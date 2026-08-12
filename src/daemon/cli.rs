use std::path::PathBuf;

pub struct DaemonConfig {
    pub ws_port: u16,
    pub ui_path: Option<PathBuf>,
    pub no_ui: bool,
    pub no_tray: bool,
    pub plugin_dirs: Vec<PathBuf>,
    pub display_backend: Option<String>,
    pub no_display: bool,
    pub restore_last: bool,
}

impl DaemonConfig {
    pub fn from_env() -> Self {
        let mut config = Self {
            ws_port: 0,
            ui_path: None,
            no_ui: false,
            no_tray: false,
            plugin_dirs: Vec::new(),
            display_backend: None,
            no_display: false,
            restore_last: true,
        };

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--ws-port" => {
                    let value = args.next().expect("--ws-port requires a value");
                    config.ws_port = value
                        .parse()
                        .expect("--ws-port must be a valid port number");
                }
                "--display-backend" => {
                    config.display_backend =
                        Some(args.next().expect("--display-backend requires a name"));
                }
                "--no-display" => config.no_display = true,
                "--ui" => {
                    config.ui_path =
                        Some(PathBuf::from(args.next().expect("--ui requires a path")));
                }
                "--no-ui" => config.no_ui = true,
                "--no-tray" => config.no_tray = true,
                "--plugin" => {
                    config.plugin_dirs.push(PathBuf::from(
                        args.next().expect("--plugin requires a path"),
                    ));
                }
                "--no-restore" => config.restore_last = false,
                other => {
                    eprintln!("unknown argument: {other}");
                    eprintln!("usage: waywallen [--ws-port PORT] [--ui PATH] [--no-ui] [--no-tray] [--plugin PATH]... [--display-backend NAME] [--no-display] [--no-restore]");
                    std::process::exit(1);
                }
            }
        }

        config
    }
}
