use super::*;
use crate::runtime::history::millis;
use rsql_core::storage::history;
use std::time::SystemTime;
mod worker;
pub(super) use worker::Read;

impl Panel {
    fn now(&self) -> i64 {
        millis(SystemTime::now())
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.disk_revision = self.session.history().revision;
        self.list.reset(0);
        self.rows.clear();
        self.databases.clear();
        self.has_more = false;
        self.load_page(None, cx);
        self.load_databases(cx);
    }

    pub(super) fn load_databases(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let project = self.form.project.clone();
        self.databases_error = None;
        self.databases_task = Some(self.read(
            move |db| async move {
                history::databases(&db, project.as_deref())
                    .await
                    .map_err(|e| e.to_string())
            },
            move |p, result, cx| p.publish_databases(generation, result, cx),
            cx,
        ));
        cx.notify();
    }

    pub(super) fn publish_databases(
        &mut self,
        generation: u64,
        result: Result<Vec<String>, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.current(generation) {
            return;
        }
        self.databases_task = None;
        match result {
            Ok(names) => self.databases = names,
            Err(error) => {
                self.report("Loading history databases failed", &error, cx);
                self.databases_error = Some(error);
            }
        }
        cx.notify();
    }

    pub(super) fn load_page(&mut self, before: Option<i64>, cx: &mut Context<Self>) {
        let filter = self.form.filter(self.now(), before);
        let generation = self.generation;
        self.page_error = None;
        self.page_task = Some(self.read(
            move |db| async move {
                let page = history::search(&db, &filter)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok((
                    page.entries.into_iter().map(Row::new).collect::<Vec<_>>(),
                    page.has_more,
                ))
            },
            move |p, result, cx| p.publish_page(generation, before, result, cx),
            cx,
        ));
        cx.notify();
    }

    pub(super) fn publish_page(
        &mut self,
        generation: u64,
        before: Option<i64>,
        result: Result<(Vec<Row>, bool), String>,
        cx: &mut Context<Self>,
    ) {
        if !self.current(generation) || self.rows.last().map(|r| r.entry.id) != before {
            return;
        }
        self.page_task = None;
        match result {
            Ok((rows, more)) => {
                let start = self.rows.len();
                self.list.splice(start..start, rows.len());
                self.rows.extend(rows);
                self.has_more = more;
            }
            Err(error) => {
                self.report("Loading history failed", &error, cx);
                self.page_error = Some(error);
            }
        }
        cx.notify();
    }
}
