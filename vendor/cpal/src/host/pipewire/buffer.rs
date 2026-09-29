//! NeonMix: SPA chunks are ring-buffer ranges, not necessarily the start of mapped memory.
/// Copy one complete interleaved chunk into preallocated, aligned storage.
/// The caller handles SPA EMPTY/CORRUPTED flags before using this helper.
#[allow(clippy::manual_is_multiple_of)] // Keep the vendored crate compatible with its Rust 1.85 MSRV.
pub(super) fn copy_chunk(
    mapped: &[u8],
    offset: usize,
    size: usize,
    frame_bytes: usize,
    scratch: &mut [u8],
) -> Result<usize, ()> {
    if frame_bytes == 0 {
        return Err(());
    }
    if mapped.is_empty() {
        return if size == 0 { Ok(0) } else { Err(()) };
    }
    let offset = offset % mapped.len();
    let size = size.min(mapped.len());
    if offset % frame_bytes != 0 || size % frame_bytes != 0 || size > scratch.len() {
        return Err(());
    }
    let first = size.min(mapped.len() - offset);
    scratch[..first].copy_from_slice(&mapped[offset..offset + first]);
    scratch[first..size].copy_from_slice(&mapped[..size - first]);
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn honors_offset_modulo_and_wrapped_chunks() {
        let mapped = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut scratch = [99; 16];
        assert_eq!(copy_chunk(&mapped, 28, 12, 4, &mut scratch), Ok(12));
        assert_eq!(&scratch[..12], &[12, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&scratch[12..], &[99; 4]);
    }
    #[test]
    fn clamps_native_size_but_rejects_partial_frames_or_short_scratch() {
        let mapped = [1u8; 16];
        let mut scratch = [99; 16];
        assert_eq!(copy_chunk(&mapped, 0, usize::MAX, 4, &mut scratch), Ok(16));
        scratch.fill(99);
        assert!(copy_chunk(&mapped, 2, 8, 4, &mut scratch).is_err());
        assert!(copy_chunk(&mapped, 0, 7, 4, &mut scratch).is_err());
        assert!(copy_chunk(&mapped, 0, 12, 4, &mut scratch[..8]).is_err());
        assert_eq!(scratch, [99; 16]);
        assert_eq!(copy_chunk(&[], 0, 0, 4, &mut scratch), Ok(0));
        assert!(copy_chunk(&[], 0, 4, 4, &mut scratch).is_err());
    }
}
