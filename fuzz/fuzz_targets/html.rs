#![no_main]

use libfuzzer_sys::fuzz_target;
use vision::{Config, Engine, Snapshot};

fuzz_target!(|data: &[u8]| {
    let Ok(html) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(engine) = Engine::native(Config {
        javascript: false,
        max_html_bytes: 256 * 1024,
        ..Config::default()
    }) else {
        return;
    };
    let _ = engine.render(&Snapshot::from_html(html));
});
