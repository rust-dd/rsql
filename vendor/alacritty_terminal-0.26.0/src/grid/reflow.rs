//! Tracks selection endpoints through the grid's row split, append and retention decisions.

use crate::index::{Column, Line, Point};

#[derive(Clone, Copy)]
enum Location {
    Grid(Point),
    Row(usize),
    Buffered(usize),
    Placed(usize, usize),
}

#[derive(Clone, Copy)]
pub(crate) struct ReflowPoint {
    location: Location,
    // A removed suffix maps to the edge after the last retained cell, never to its contents.
    pub(crate) after_cell: bool,
}

impl ReflowPoint {
    pub(crate) fn new(point: Point) -> Self {
        Self {
            location: Location::Grid(point),
            after_cell: false,
        }
    }

    pub(crate) fn mapped(self) -> Option<Point> {
        match self.location {
            Location::Grid(point) => Some(point),
            _ => None,
        }
    }
}

pub(super) struct Tracks<'a>(pub &'a mut [ReflowPoint]);

impl Tracks<'_> {
    pub fn clip_source(&mut self, first: Line, last: Line) {
        for point in &mut *self.0 {
            if let Location::Grid(p) = &mut point.location {
                // Height changes can remove a row before the subsequent width reflow.
                if p.line < first {
                    p.line = Line(i32::MIN);
                } else if p.line > last {
                    p.line = Line(i32::MAX);
                }
            }
        }
    }

    pub fn begin_row(&mut self, line: Line, prepended: usize) {
        for point in &mut *self.0 {
            match point.location {
                Location::Grid(p) if p.line == line => {
                    point.location = Location::Row(prepended + p.column.0);
                }
                Location::Buffered(column) => point.location = Location::Row(column),
                _ => (),
            }
        }
    }

    pub fn place_prefix(&mut self, row: usize, offset: usize, length: usize) {
        for point in &mut *self.0 {
            if let Location::Row(column) = point.location {
                point.location = if column < length {
                    Location::Placed(row, offset + column)
                } else {
                    Location::Row(column - length)
                };
            }
        }
    }

    pub fn clip_columns(&mut self, retained: usize) {
        for point in &mut *self.0 {
            if let Location::Row(column) = &mut point.location
                && *column >= retained
            {
                *column = retained - 1;
                point.after_cell = true;
            }
        }
    }

    pub fn finish_row(&mut self, row: usize, last_column: usize) {
        self.clip_columns(last_column + 1);
        for point in &mut *self.0 {
            if let Location::Row(column) = point.location {
                point.location = Location::Placed(row, column);
            }
        }
    }

    pub fn discard_suffix(&mut self, row: usize, last_column: usize) {
        for point in &mut *self.0 {
            if matches!(point.location, Location::Row(_)) {
                point.after_cell = true;
                point.location = Location::Placed(row, last_column);
            }
        }
    }

    pub fn buffer(&mut self) {
        for point in &mut *self.0 {
            if let Location::Row(column) = point.location {
                point.location = Location::Buffered(column);
            }
        }
    }

    pub fn finish(&mut self, first_line: i32) {
        for point in &mut *self.0 {
            if let Location::Placed(row, column) = point.location {
                point.location =
                    Location::Grid(Point::new(Line(first_line + row as i32), Column(column)));
            }
        }
    }
}
