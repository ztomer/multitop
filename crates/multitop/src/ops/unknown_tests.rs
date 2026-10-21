//! **A CALL THAT MISSED THE DEADLINE IS A GAP IN ONE SECTION, NOT A FAULT
//! OF THE ESTATE.**
//!
//! servers ROADMAP E74: the local host's `docker.containers` call missed
//! the estate's 20-second deadline on 17 of about 125 passes, and the poll
//! read that as a failed tool - a fault of the estate, and the whole
//! host's reading thrown away with it.
//!
//! So the section that could not be read reads UNKNOWN, says which call
//! missed the deadline, and the sections that were read stand.

use std::time::Duration;

use super::render::render;
use super::tests::{host, pal, plain};
use super::*;
use crate::mcp::client::Client;
use crate::mcp::fake::{serve, Reply};

const NOW: i64 = 1_790_170_000;

#[tokio::test]
async fn a_call_that_missed_the_deadline_reads_unknown_and_the_rest_stand() {
    // The local host: launchd's cron and Docker. The docker call does not
    // answer in time.
    let (rd, wr, _) = serve(host(&["cron.status", "docker.containers"], Reply::Silent));
    let mut client = Client::connect(rd, wr, Duration::from_millis(200))
        .await
        .unwrap();
    let s = gather(&mut client, NOW, Previous::default()).await;
    let f = plain(&render(
        "127.0.0.1",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.iter()
            .any(|l| l.contains("containers unknown: docker.containers: no answer in 200ms")),
        "the section that could not be read does not say so: {f:?}"
    );
    assert!(
        f.iter().any(|l| l.contains("cron ")),
        "the readings that were taken were thrown away with the timed-out one: {f:?}"
    );
}
