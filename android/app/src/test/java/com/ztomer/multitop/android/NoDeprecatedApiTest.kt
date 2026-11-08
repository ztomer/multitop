package com.ztomer.multitop.android

import com.ztomer.multitop.android.push.FcmService
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.io.File
import java.lang.reflect.AnnotatedElement
import java.lang.reflect.Method
import java.net.URI
import java.util.jar.JarFile

/**
 * No deprecated API in the app (engineering rule 17), the half the compiler
 * cannot hold. `allWarningsAsErrors` already fails a CALL to a deprecated API
 * and an override of one, unless the override is itself marked `@Deprecated`:
 * that annotation silences the warning, and it is how Firebase 26's deprecated
 * `onNewToken` sat in FcmService with the build green (M.FCMDEPR). So this
 * scans every class the app compiles and fails on (1) any declaration marked
 * deprecated and (2) any method that overrides a deprecated one.
 */
class NoDeprecatedApiTest {
    private val loader = FcmService::class.java.classLoader!!

    /** Every class in the app's own compiled main classes, by name. */
    private fun appClasses(): List<Class<*>> {
        // Located through the class loader: JDK 27 deprecates ProtectionDomain.getCodeSource.
        val anchor = FcmService::class.java.name.replace('.', '/') + ".class"
        val url = loader.getResource(anchor) ?: error("no $anchor on the test classpath")
        val names =
            when (url.protocol) {
                "file" -> {
                    val root = File(url.toURI()).path.removeSuffix(anchor).let(::File)
                    root
                        .walk()
                        .filter { it.extension == "class" }
                        .map { it.relativeTo(root).invariantSeparatorsPath }
                        .toList()
                }

                "jar" -> {
                    val jar = File(URI(url.path.substringBefore("!/")))
                    JarFile(jar).use { j ->
                        j
                            .entries()
                            .asSequence()
                            .map { it.name }
                            .filter { it.endsWith(".class") }
                            .toList()
                    }
                }

                else -> {
                    error("app classes at an unreadable $url")
                }
            }
        return names
            .map { it.removeSuffix(".class").replace('/', '.') }
            .filter { it.startsWith("com.ztomer.multitop.android.") }
            .map { Class.forName(it, false, loader) }
    }

    // By name: Kotlin warns on naming java.lang.Deprecated, which it maps to its own.
    private val deprecations = setOf("java.lang.Deprecated", "kotlin.Deprecated")

    private fun AnnotatedElement.deprecated() = declaredAnnotations.any { it.annotationClass.java.name in deprecations }

    private fun supertypes(c: Class<*>): Sequence<Class<*>> =
        (listOfNotNull(c.superclass) + c.interfaces).asSequence().flatMap { sequenceOf(it) + supertypes(it) }

    private fun overridesDeprecated(m: Method): Boolean =
        supertypes(m.declaringClass).any { s ->
            runCatching { s.getDeclaredMethod(m.name, *m.parameterTypes) }.getOrNull()?.deprecated() == true
        }

    private fun findings(classes: List<Class<*>>): List<String> =
        classes.flatMap { c ->
            val own = if (c.deprecated()) listOf("${c.name} is marked deprecated") else emptyList()
            own +
                c.declaredMethods.filterNot { it.isSynthetic || it.isBridge }.flatMap { m ->
                    listOfNotNull(
                        "${c.name}.${m.name} is marked deprecated".takeIf { m.deprecated() },
                        "${c.name}.${m.name} overrides a deprecated API".takeIf { overridesDeprecated(m) },
                    )
                }
        }

    @Test
    fun `the scan sees the app, and sees a deprecated override`() {
        // Calibration: an instrument that loaded nothing would pass anything.
        val classes = appClasses()
        assertTrue(FcmService::class.java in classes, "the scan did not find FcmService in ${classes.size} classes")
        assertTrue(classes.size > 20, "the scan found only ${classes.size} app classes")
        val seeded = Seeded::class.java.name
        assertEquals(
            listOf("$seeded.onNewToken is marked deprecated", "$seeded.onNewToken overrides a deprecated API"),
            findings(listOf(Seeded::class.java)),
        )
    }

    @Test
    fun `no class in the app declares or overrides a deprecated API`() {
        assertEquals(emptyList<String>(), findings(appClasses()))
    }
}

/**
 * The shape the scan must refuse -- exactly what FcmService carried before
 * M.FCMDEPR. A test class, so the app scan never sees it.
 */
private class Seeded : com.google.firebase.messaging.FirebaseMessagingService() {
    @Deprecated("the seeded fault", ReplaceWith("onRegistered(token)"))
    override fun onNewToken(token: String) = Unit
}
