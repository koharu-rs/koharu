use indoc::formatdoc;

/// Chains after Sentry's hook, which reports and sends the panic, then tells
/// the user and closes the app, so a panic on any thread doesn't leave it
/// running.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);

        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic");
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();

        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Koharu has stopped")
            .set_description(formatdoc! {"
                Something went wrong, and Koharu needs to close.

                {msg}
                at {location}"
            })
            .set_buttons(rfd::MessageButtons::Ok)
            .show();

        std::process::exit(1);
    }));
}
