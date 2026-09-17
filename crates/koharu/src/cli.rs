use clap::Parser;

#[derive(Parser)]
#[command(version, about)]
pub struct Cli {
    #[arg(short, long, help = "Download dynamic libraries and exit")]
    pub(crate) download: bool,
    #[arg(long, help = "Force CPU even if GPU is available")]
    pub(crate) cpu: bool,
    #[arg(short, long, value_name = "PORT", help = "Bind to a specific port")]
    pub(crate) port: Option<u16>,
    #[arg(
        long,
        help = "Bind the HTTP service to a specific host instead of 127.0.0.1"
    )]
    pub(crate) host: Option<String>,
    #[arg(long, help = "Run without GUI")]
    pub(crate) headless: bool,
    #[arg(long, help = "Enable debug console output")]
    pub(crate) debug: bool,
}

impl Cli {
    pub(crate) fn tracing_level(&self) -> tracing::Level {
        if self.debug {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser as _;
    #[test]
    fn headless_port_host_cpu_download_debug() {
        let cli = Cli::try_parse_from([
            "koharu",
            "--headless",
            "--port",
            "4000",
            "--host",
            "0.0.0.0",
            "--cpu",
            "--download",
            "--debug",
        ])
        .unwrap();
        assert!(cli.headless && cli.cpu && cli.download && cli.debug);
        assert_eq!(cli.port, Some(4000));
        assert_eq!(cli.host.as_deref(), Some("0.0.0.0"));
        assert_eq!(cli.tracing_level(), tracing::Level::DEBUG);
    }

    #[test]
    fn defaults_are_gui() {
        let cli = Cli::try_parse_from(["koharu"]).unwrap();
        assert!(!cli.headless && !cli.cpu && !cli.download && !cli.debug);
        assert!(cli.port.is_none() && cli.host.is_none());
        assert_eq!(cli.tracing_level(), tracing::Level::INFO);
    }
}
