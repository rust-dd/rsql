use std::{ops::Index, sync::Arc};

/// Ordered draft rows. Published revisions share pages until a row on that page changes.
#[derive(Clone, Debug)]
pub struct DraftRows<T> {
    pages: Arc<Vec<Arc<Vec<T>>>>,
    len: usize,
}
const PAGE_SIZE: usize = 32;
impl<T> Default for DraftRows<T> {
    fn default() -> Self {
        Self {
            pages: Default::default(),
            len: 0,
        }
    }
}
impl<T> DraftRows<T> {
    pub fn iter(&self) -> impl Iterator<Item = &T> + Clone {
        self.into_iter()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn locate(&self, mut index: usize) -> (usize, usize) {
        for (page, rows) in self.pages.iter().enumerate() {
            if index < rows.len() {
                return (page, index);
            }
            index -= rows.len();
        }
        panic!("draft row index out of bounds");
    }
}
impl<T: Clone> DraftRows<T> {
    pub(super) fn push(&mut self, value: T) {
        let pages = Arc::make_mut(&mut self.pages);
        if pages.last().is_none_or(|page| page.len() == PAGE_SIZE) {
            pages.push(Arc::new(Vec::with_capacity(PAGE_SIZE)));
        }
        Arc::make_mut(pages.last_mut().unwrap()).push(value);
        self.len += 1;
    }
    pub(super) fn edit(&mut self, index: usize) -> &mut T {
        let (page, i) = self.locate(index);
        &mut Arc::make_mut(&mut Arc::make_mut(&mut self.pages)[page])[i]
    }
    pub(super) fn remove(&mut self, index: usize) {
        let (page, i) = self.locate(index);
        let pages = Arc::make_mut(&mut self.pages);
        let rows = Arc::make_mut(&mut pages[page]);
        rows.remove(i);
        if rows.is_empty() {
            pages.remove(page);
        }
        self.len -= 1;
    }
}
impl<T> Index<usize> for DraftRows<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        let (page, i) = self.locate(index);
        &self.pages[page][i]
    }
}
impl<'a, T> IntoIterator for &'a DraftRows<T> {
    type Item = &'a T;
    type IntoIter = std::iter::FlatMap<
        std::slice::Iter<'a, Arc<Vec<T>>>,
        &'a [T],
        fn(&'a Arc<Vec<T>>) -> &'a [T],
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.pages.iter().flat_map(|page| page.as_slice())
    }
}
impl<T: Clone> FromIterator<T> for DraftRows<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        let mut rows = Self::default();
        for item in items {
            rows.push(item);
        }
        rows
    }
}
