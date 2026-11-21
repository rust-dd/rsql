use crate::TestSupportExt as _;
use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement, Interactivity, IntoElement, ParentElement,
    RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement, Styled,
    Window, div, prelude::FluentBuilder as _,
};

use crate::StyledExt as _;

macro_rules! table_part {
    ($name:ident, $role:expr, $docs:literal) => {
        #[doc = $docs]
        #[derive(IntoElement)]
        pub struct $name {
            base: crate::ObservedElement<Stateful<Div>>,
            style: StyleRefinement,
            children: Vec<AnyElement>,
        }

        impl $name {
            #[doc = concat!("Create ", $docs)]
            pub fn new(id: impl Into<ElementId>) -> Self {
                Self {
                    base: div().id(id).test_support(),
                    style: StyleRefinement::default(),
                    children: Vec::new(),
                }
            }
        }

        impl Styled for $name {
            fn style(&mut self) -> &mut StyleRefinement {
                &mut self.style
            }
        }

        impl ParentElement for $name {
            fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
                self.children.extend(children);
            }
        }

        impl InteractiveElement for $name {
            fn interactivity(&mut self) -> &mut Interactivity {
                self.base.interactivity()
            }

            fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
                self.base = self.base.track_focus(handle);
                self
            }
        }

        impl StatefulInteractiveElement for $name {}

        impl RenderOnce for $name {
            fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
                self.base
                    .role($role)
                    .children(self.children)
                    .refine_style(&self.style)
            }
        }
    };
}

/// An unstyled semantic table root.
#[derive(IntoElement)]
pub struct Table {
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    row_count: Option<usize>,
    column_count: Option<usize>,
    accessibility_label: Option<SharedString>,
}

impl Table {
    /// Create an unstyled semantic table root.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            style: StyleRefinement::default(),
            children: Vec::new(),
            row_count: None,
            column_count: None,
            accessibility_label: None,
        }
    }

    /// Sets the total number of rows, including rows outside the rendered
    /// range. Assistive technology needs this to announce "row 5 of 200" for a
    /// virtualized table whose rendered rows are only a window onto the data.
    pub fn row_count(mut self, count: usize) -> Self {
        self.row_count = Some(count);
        self
    }

    /// Sets the total number of columns, including columns outside the
    /// rendered range.
    pub fn column_count(mut self, count: usize) -> Self {
        self.column_count = Some(count);
        self
    }

    /// Sets the table's accessible name.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }
}

impl Styled for Table {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Table {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}

impl InteractiveElement for Table {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for Table {}

impl RenderOnce for Table {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .role(Role::Table)
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            .when_some(self.row_count, |this, count| this.aria_row_count(count))
            .when_some(self.column_count, |this, count| {
                this.aria_column_count(count)
            })
            .children(self.children)
            .refine_style(&self.style)
    }
}
table_part!(
    TableHeader,
    Role::RowGroup,
    "An unstyled table header group."
);
table_part!(TableBody, Role::RowGroup, "An unstyled table body group.");

/// An unstyled semantic table row.
#[derive(IntoElement)]
pub struct TableRow {
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    row_index: usize,
    children: Vec<AnyElement>,
}

impl TableRow {
    /// Create an unstyled semantic table row with a one-based accessibility index.
    pub fn new(id: impl Into<ElementId>, row_index: usize) -> Self {
        Self {
            base: div().id(id).test_support(),
            style: StyleRefinement::default(),
            row_index,
            children: Vec::new(),
        }
    }
}

impl Styled for TableRow {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for TableRow {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}

impl InteractiveElement for TableRow {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for TableRow {}

impl RenderOnce for TableRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .role(Role::Row)
            .aria_row_index(self.row_index)
            .children(self.children)
            .refine_style(&self.style)
    }
}

macro_rules! table_cell {
    ($name:ident, $role:expr, $docs:literal) => {
        #[doc = $docs]
        #[derive(IntoElement)]
        pub struct $name {
            base: crate::ObservedElement<Stateful<Div>>,
            style: StyleRefinement,
            column_index: usize,
            children: Vec<AnyElement>,
        }

        impl $name {
            #[doc = concat!("Create ", $docs, " with a one-based accessibility index.")]
            pub fn new(id: impl Into<ElementId>, column_index: usize) -> Self {
                Self {
                    base: div().id(id).test_support(),
                    style: StyleRefinement::default(),
                    column_index,
                    children: Vec::new(),
                }
            }
        }

        impl Styled for $name {
            fn style(&mut self) -> &mut StyleRefinement {
                &mut self.style
            }
        }

        impl ParentElement for $name {
            fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
                self.children.extend(children);
            }
        }

        impl InteractiveElement for $name {
            fn interactivity(&mut self) -> &mut Interactivity {
                self.base.interactivity()
            }

            fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
                self.base = self.base.track_focus(handle);
                self
            }
        }

        impl StatefulInteractiveElement for $name {}

        impl RenderOnce for $name {
            fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
                self.base
                    .role($role)
                    .aria_column_index(self.column_index)
                    .children(self.children)
                    .refine_style(&self.style)
            }
        }
    };
}

table_cell!(
    TableHead,
    Role::ColumnHeader,
    "An unstyled table column header."
);
table_cell!(TableCell, Role::Cell, "An unstyled table data cell.");

/// An unstyled table caption slot.
#[derive(IntoElement)]
pub struct TableCaption {
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl TableCaption {
    /// Create an unstyled table caption slot.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }
}

impl Styled for TableCaption {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for TableCaption {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}

impl InteractiveElement for TableCaption {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for TableCaption {}

impl RenderOnce for TableCaption {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base.children(self.children).refine_style(&self.style)
    }
}
