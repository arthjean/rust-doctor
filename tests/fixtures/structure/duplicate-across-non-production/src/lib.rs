







pub fn shipped(values: &[u32]) -> u32 {
    values.iter().copied().max().unwrap_or_default()
}
