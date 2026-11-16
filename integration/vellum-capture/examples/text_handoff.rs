//! Write one private fixture handoff to a new user-selected directory. No live
//! data, application, engine service or provider is required to read the output.
#[path = "../tests/text_handoff.rs"]
#[cfg(not(test))]
mod fixture;
#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = args.next().ok_or("usage: text_handoff NEW_DIRECTORY")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    fixture::write_fixture_handoff(std::path::Path::new(&destination))?;
    println!(
        "Fixture handoff created: edit work.org with any text editor; baseline.org preserves the original; handoff.txt explains the last observed state. Offline edits grant no execution authority."
    );
    Ok(())
}
#[cfg(test)]
fn main() {}
