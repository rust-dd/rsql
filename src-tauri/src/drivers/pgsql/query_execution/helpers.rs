use tokio_postgres::{SimpleColumn, SimpleQueryMessage};

use super::super::wire::Cell;

/// Return the last rowset, preserving its columns even when it has no rows.
/// Scripts without rowsets return an affected-row message when the count is positive.
pub(crate) fn process_simple_messages(
    messages: Vec<SimpleQueryMessage>,
) -> (Vec<String>, Vec<Vec<Cell>>) {
    let mut current = None::<(Vec<String>, Vec<Vec<Cell>>)>;
    let mut last = None;
    let mut total_affected = 0u64;

    for msg in messages {
        match msg {
            SimpleQueryMessage::RowDescription(columns) => {
                current = Some((column_names(&columns), Vec::new()));
            }
            SimpleQueryMessage::Row(row) => {
                let (_, rows) =
                    current.get_or_insert_with(|| (column_names(row.columns()), Vec::new()));
                rows.push(row_cells(&row));
            }
            SimpleQueryMessage::CommandComplete(n) => {
                if let Some(result) = current.take() {
                    last = Some(result);
                } else {
                    total_affected += n;
                }
            }
            _ => {}
        }
    }

    if let Some(result) = current.or(last) {
        result
    } else if total_affected > 0 {
        (
            vec!["Result".into()],
            vec![vec![Some(format!("{} rows affected", total_affected))]],
        )
    } else {
        (Vec::new(), Vec::new())
    }
}

/// Column names from a simple-query result description, in result order.
pub(crate) fn column_names(columns: &[SimpleColumn]) -> Vec<String> {
    columns.iter().map(|c| c.name().to_owned()).collect()
}

/// Cell values of a simple-query row. `None` is SQL NULL.
pub(crate) fn row_cells(row: &tokio_postgres::SimpleQueryRow) -> Vec<Cell> {
    let col_count = row.columns().len();
    let mut cells = Vec::with_capacity(col_count);
    for i in 0..col_count {
        cells.push(row.get(i).map(str::to_owned));
    }
    cells
}
