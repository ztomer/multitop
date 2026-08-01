use crate::config::Server;
use multitop_agent::fetch::FetchSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Fetch,
    Upgrade,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpgradeState {
    #[default]
    NIL,
    STARTED,
    DONE,
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub server: Server,
    pub mode: Mode,
    pub gen: u64,
    pub last_frame: Option<Vec<String>>,
    pub last_fetch: Option<FetchSnapshot>,
    pub last_upgrade: Vec<String>,
    pub upgrade_state: UpgradeState,
    pub upgrade_gen: u64,
    pub last_monitor: Option<multitop_agent::proto::Payload>,
    pub last_docker: Option<multitop_agent::proto::Payload>,
    pub view: Vec<String>,
    pub scroll_offset: usize,
    pub sudo_password: Option<String>,
    pub password_saved: bool,
    pub external_password: bool,
}

impl Panel {
    #[must_use]
    pub fn new(server: Server) -> Self {
        let pal = &multitop_agent::color::ANSI;
        Self {
            server,
            mode: Mode::Monitor,
            gen: 0,
            last_frame: None,
            last_fetch: None,
            last_upgrade: Vec::new(),
            upgrade_state: UpgradeState::NIL,
            upgrade_gen: 0,
            last_monitor: None,
            last_docker: None,
            view: vec![format!("{}connecting...{}", pal.muted(), pal.reset)],
            scroll_offset: 0,
            sudo_password: None,
            password_saved: false,
            external_password: false,
        }
    }

    pub fn ensure_sudo_password(&mut self) -> Option<String> {
        if self.sudo_password.is_none() {
            if let Ok(Some(pass)) = crate::password_store::load(&self.server) {
                self.sudo_password = Some(pass);
                self.password_saved = true;
            }
        }
        self.sudo_password.clone()
    }

    pub fn set_sudo_password(&mut self, password: String, from_vault: bool) {
        self.sudo_password = Some(password);
        if from_vault {
            self.external_password = true;
        }
    }

    pub fn show_last_frame(&mut self) {
        let pal = &multitop_agent::color::ANSI;
        self.view = match &self.last_frame {
            Some(f) => f.clone(),
            None => vec![format!(
                "{}waiting for data...{}",
                pal.meter_mid(),
                pal.reset
            )],
        };
    }
}
