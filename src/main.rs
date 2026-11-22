/// Building the clap command tree takes a lot of stack in debug builds, and
/// Windows gives the main thread only 1 MiB, so run the CLI on its own thread.
const STACK_SIZE: usize = 8 * 1024 * 1024;

fn main() {
    let cli = std::thread::Builder::new()
        .name("atlx".into())
        .stack_size(STACK_SIZE)
        .spawn(atlx::cli::main)
        .expect("cannot start the CLI thread");
    if let Err(panic) = cli.join() {
        std::panic::resume_unwind(panic);
    }
}
