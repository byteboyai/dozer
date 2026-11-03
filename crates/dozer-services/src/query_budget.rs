//! Bound query materialization before constructing/encoding a full reply.
use crate::{ServiceError, ServiceErrorKind};
pub const MAX_ROWS: usize = 10_000;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub fn collect<T, E: std::fmt::Display>(
    rows: impl IntoIterator<Item = Result<T, E>>,
    heap_bytes: impl Fn(&T) -> usize,
) -> Result<Vec<T>, ServiceError> {
    collect_with_limits(rows, heap_bytes, MAX_ROWS, MAX_BYTES)
}
pub fn collect_with_limits<T, E: std::fmt::Display>(
    rows: impl IntoIterator<Item = Result<T, E>>,
    heap_bytes: impl Fn(&T) -> usize,
    max_rows: usize,
    max_bytes: usize,
) -> Result<Vec<T>, ServiceError> {
    let mut result = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let row = row.map_err(|e| ServiceError::new(ServiceErrorKind::Rejected, e.to_string()))?;
        bytes = bytes
            .saturating_add(std::mem::size_of::<T>())
            .saturating_add(heap_bytes(&row));
        if result.len() >= max_rows || bytes > max_bytes {
            return Err(ServiceError::new(
                ServiceErrorKind::Rejected,
                "Limit: query result exceeds row/byte budget; use a smaller page",
            ));
        }
        result.push(row);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_and_byte_limits_fail_instead_of_returning_a_partial_list() {
        assert!(collect_with_limits((0..4).map(Ok::<_, String>), |_| 0, 3, 1024).is_err());
        assert!(
            collect_with_limits([Ok::<_, String>("payload".to_string())], |s| s.len(), 10, 1)
                .is_err()
        );
        assert_eq!(
            collect_with_limits([Ok::<_, String>(1)], |_| 0, 1, 64).unwrap(),
            [1]
        );
    }
}
