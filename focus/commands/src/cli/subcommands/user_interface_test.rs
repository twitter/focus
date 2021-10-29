use std::{
    sync::Arc,
    thread::{spawn, JoinHandle},
    time::Duration,
};

use anyhow::Result;

use focus_internals::{app::App, ui::ProgressReporter};

pub fn run(app: Arc<App>) -> Result<()> {
    let sandbox = app.sandbox();

    let mut handles = Vec::<JoinHandle<()>>::new();
    for i in 1..8 {
        let cloned_app = app.clone();
        let handle = std::thread::spawn(move || {
            let cloned_app_for_reporter = cloned_app.clone();
            let _report =
                ProgressReporter::new(cloned_app_for_reporter.clone(), format!("Thread {}", i))
                    .expect("Instantiating progress reporter failed");

            {
                std::thread::sleep(Duration::from_secs(i));
            }
        });
        handles.push(handle);
    }

    let cloned_sandbox_for_beer_logger = sandbox.clone();
    let cloned_ui = app.ui();
    let _beer_thread = spawn(move || {
        let cloned_ui = cloned_ui.clone();
        let _cloned_sandbox_for_beer_logger = cloned_sandbox_for_beer_logger.clone();
        for i in (0..999).rev() {
            cloned_ui.log(
                    format!("Shakespeare's Cousin"),
                    format!(
                        "{} bottles of beer on the wall, {} bottles of beer, take one down, pass it around, {} bottles of beer on the wall",
                        i,
                        i,
                        i - 1
                    ),
                );
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    let cloned_ui = app.ui();
    for handle in handles {
        cloned_ui.log(
            format!("Thread Butler"),
            format!("Waiting on thread {:?}", handle.thread()),
        );
        handle.join().expect("Thread failed");
    }

    Ok(())
}
