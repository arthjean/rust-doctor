//! Feature-gated Language Server Protocol transport.

mod analysis;
mod server;

/// Adapter protocol negotiated independently from the binary package version.
pub const PROTOCOL_MAJOR: u32 = 1;

#[derive(thiserror::Error, Debug)]
pub enum LspError {
    #[error("failed to create the LSP runtime: {0}")]
    Runtime(#[from] std::io::Error),
}

pub fn run() -> Result<(), LspError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(server::serve());
    Ok(())
}
