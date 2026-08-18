//! Reading a child process's stream without letting it decide how much memory
//! this process spends.
//!
//! Two layers shell out: `git` runs the porcelain every scope, baseline and
//! repository pass needs, and `execution` runs Cargo. Both read from a pipe a
//! scanned workspace can make arbitrarily large, so both read through here
//! rather than through `Command::output`, which has no bound at all.

use std::io::{self, Read};

/// What a bounded read kept, and whether the stream had more to give.
#[derive(Debug)]
pub(crate) struct BoundedOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) exceeded: bool,
}

/// Reads a stream to its end, keeping at most `limit` bytes.
///
/// The read continues past the limit rather than stopping at it, because a
/// producer blocked on a full pipe never exits and the caller is waiting on it.
pub(crate) fn collect_bounded(mut reader: impl Read, limit: usize) -> io::Result<BoundedOutput> {
    let mut bytes = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0_u8; 8_192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if !exceeded {
            let kept = limit.saturating_sub(bytes.len()).min(read);
            bytes.extend(buffer.iter().take(kept).copied());
            exceeded = kept < read;
        }
    }
    Ok(BoundedOutput { bytes, exceeded })
}
