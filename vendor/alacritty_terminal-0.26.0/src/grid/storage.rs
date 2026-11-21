// RSQL modification: complete row swaps; see NOTICE.
use std::cmp::max;
use std::mem;
use std::ops::{Index, IndexMut};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Row;
use crate::index::Line;

/// Maximum number of buffered lines outside of the grid for performance optimization.
const MAX_CACHE_SIZE: usize = 1_000;

/// A ring buffer for optimizing indexing and rotation.
///
/// The [`Storage::rotate`] and [`Storage::rotate_down`] functions are fast modular additions on
/// the internal [`zero`] field. As compared with [`slice::rotate_left`] which must rearrange items
/// in memory.
///
/// As a consequence, both [`Index`] and [`IndexMut`] are reimplemented for this type to account
/// for the zeroth element not always being at the start of the allocation.
///
/// Because certain [`Vec`] operations are no longer valid on this type, no [`Deref`]
/// implementation is provided. Anything from [`Vec`] that should be exposed must be done so
/// manually.
///
/// [`slice::rotate_left`]: https://doc.rust-lang.org/std/primitive.slice.html#method.rotate_left
/// [`Deref`]: std::ops::Deref
/// [`zero`]: #structfield.zero
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Storage<T> {
    inner: Vec<Row<T>>,

    /// Starting point for the storage of rows.
    ///
    /// This value represents the starting line offset within the ring buffer. The value of this
    /// offset may be larger than the `len` itself, and will wrap around to the start to form the
    /// ring buffer. It represents the bottommost line of the terminal.
    zero: usize,

    /// Number of visible lines.
    visible_lines: usize,

    /// Total number of lines currently active in the terminal (scrollback + visible)
    ///
    /// Shrinking this length allows reducing the number of lines in the scrollback buffer without
    /// having to truncate the raw `inner` buffer.
    /// As long as `len` is bigger than `inner`, it is also possible to grow the scrollback buffer
    /// without any additional insertions.
    len: usize,
}

impl<T: PartialEq> PartialEq for Storage<T> {
    fn eq(&self, other: &Self) -> bool {
        // Both storage buffers need to be truncated and zeroed.
        assert_eq!(self.zero, 0);
        assert_eq!(other.zero, 0);

        self.inner == other.inner && self.len == other.len
    }
}

impl<T> Storage<T> {
    #[inline]
    pub fn with_capacity(visible_lines: usize, columns: usize) -> Storage<T>
    where
        T: Default,
    {
        // Initialize visible lines; the scrollback buffer is initialized dynamically.
        let mut inner = Vec::with_capacity(visible_lines);
        inner.resize_with(visible_lines, || Row::new(columns));

        Storage { inner, zero: 0, visible_lines, len: visible_lines }
    }

    /// Increase the number of lines in the buffer.
    #[inline]
    pub fn grow_visible_lines(&mut self, next: usize)
    where
        T: Default,
    {
        // Number of lines the buffer needs to grow.
        let additional_lines = next - self.visible_lines;

        let columns = self[Line(0)].len();
        self.initialize(additional_lines, columns);

        // Update visible lines.
        self.visible_lines = next;
    }

    /// Decrease the number of lines in the buffer.
    #[inline]
    pub fn shrink_visible_lines(&mut self, next: usize) {
        // Shrink the size without removing any lines.
        let shrinkage = self.visible_lines - next;
        self.shrink_lines(shrinkage);

        // Update visible lines.
        self.visible_lines = next;
    }

    /// Shrink the number of lines in the buffer.
    #[inline]
    pub fn shrink_lines(&mut self, shrinkage: usize) {
        self.len -= shrinkage;

        // Free memory.
        if self.inner.len() > self.len + MAX_CACHE_SIZE {
            self.truncate();
        }
    }

    /// Truncate the invisible elements from the raw buffer.
    #[inline]
    pub fn truncate(&mut self) {
        self.rezero();

        self.inner.truncate(self.len);
    }

    /// Dynamically grow the storage buffer at runtime.
    #[inline]
    pub fn initialize(&mut self, additional_rows: usize, columns: usize)
    where
        T: Default,
    {
        if self.len + additional_rows > self.inner.len() {
            self.rezero();

            let realloc_size = self.inner.len() + max(additional_rows, MAX_CACHE_SIZE);
            self.inner.resize_with(realloc_size, || Row::new(columns));
        }

        self.len += additional_rows;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn swap(&mut self, a: Line, b: Line) {
        let a = self.compute_index(a);
        let b = self.compute_index(b);
        self.inner.swap(a, b);
    }

    /// Rotate the grid, moving all lines up/down in history.
    #[inline]
    pub fn rotate(&mut self, count: isize) {
        debug_assert!(count.unsigned_abs() <= self.inner.len());

        let len = self.inner.len();
        self.zero = (self.zero as isize + count + len as isize) as usize % len;
    }

    /// Rotate all existing lines down in history.
    ///
    /// This is a faster, specialized version of [`rotate_left`].
    ///
    /// [`rotate_left`]: https://doc.rust-lang.org/std/vec/struct.Vec.html#method.rotate_left
    #[inline]
    pub fn rotate_down(&mut self, count: usize) {
        self.zero = (self.zero + count) % self.inner.len();
    }

    /// Update the raw storage buffer.
    #[inline]
    pub fn replace_inner(&mut self, vec: Vec<Row<T>>) {
        self.len = vec.len();
        self.inner = vec;
        self.zero = 0;
    }

    /// Remove all rows from storage.
    #[inline]
    pub fn take_all(&mut self) -> Vec<Row<T>> {
        self.truncate();

        let mut buffer = Vec::new();

        mem::swap(&mut buffer, &mut self.inner);
        self.len = 0;

        buffer
    }

    /// Compute actual index in underlying storage given the requested index.
    #[inline]
    fn compute_index(&self, requested: Line) -> usize {
        debug_assert!(requested.0 < self.visible_lines as i32);

        let positive = -(requested - self.visible_lines).0 as usize - 1;

        debug_assert!(positive < self.len);

        let zeroed = self.zero + positive;

        // Use if/else instead of remainder here to improve performance.
        //
        // Requires `zeroed` to be smaller than `self.inner.len() * 2`,
        // but both `self.zero` and `requested` are always smaller than `self.inner.len()`.
        if zeroed >= self.inner.len() { zeroed - self.inner.len() } else { zeroed }
    }

    /// Rotate the ringbuffer to reset `self.zero` back to index `0`.
    #[inline]
    fn rezero(&mut self) {
        if self.zero == 0 {
            return;
        }

        self.inner.rotate_left(self.zero);
        self.zero = 0;
    }
}

impl<T> Index<Line> for Storage<T> {
    type Output = Row<T>;

    #[inline]
    fn index(&self, index: Line) -> &Self::Output {
        let index = self.compute_index(index);
        &self.inner[index]
    }
}

impl<T> IndexMut<Line> for Storage<T> {
    #[inline]
    fn index_mut(&mut self, index: Line) -> &mut Self::Output {
        let index = self.compute_index(index);
        &mut self.inner[index]
    }
}
