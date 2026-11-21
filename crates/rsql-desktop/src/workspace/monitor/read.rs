use super::{catalog, model::*};
use rsql_core::{ApplicationServices, Error, postgres::statistics_activity as pg};
use tokio::sync::watch;

fn typed<T: TryFrom<Vec<String>, Error = Error>>(raw: Vec<Vec<String>>) -> Result<Vec<T>, Error> {
    raw.into_iter().map(T::try_from).collect()
}

fn advised<T: TryFrom<Vec<String>, Error = Error>>(
    raw: Vec<Vec<String>>,
    flagged: impl Fn(&T) -> bool,
    advice: impl FnOnce(usize) -> String,
) -> Result<Advised<T>, Error> {
    let mut rows = Vec::with_capacity(raw.len());
    let mut count = 0;
    for fields in raw {
        let row = T::try_from(fields)?;
        count += usize::from(flagged(&row));
        rows.push(row);
    }
    Ok(Advised {
        rows,
        note: (count > 0).then(|| advice(count).into()),
    })
}

impl Rows {
    pub(super) fn locks(raw: Vec<Vec<String>>) -> Result<Self, Error> {
        advised(
            raw,
            |row: &Lock| row.status == "waiting",
            |n| format!("Waiting locks: {n}"),
        )
        .map(Self::Locks)
    }

    pub(super) fn indexes(raw: Vec<Vec<String>>) -> Result<Self, Error> {
        advised(raw, |row: &IndexUsage| row.status == "unused", |n| format!(
            "Unused indexes: {n}. Consider removing them to save space and improve write performance."
        )).map(Self::Indexes)
    }

    pub(super) fn bloat(raw: Vec<Vec<String>>) -> Result<Self, Error> {
        advised(
            raw,
            |row: &Bloat| matches!(bloat_tone(number(&row.pct)), Tone::Warning | Tone::Danger),
            |n| format!("Tables with > 10% bloat: {n}. Consider running VACUUM."),
        )
        .map(Self::Bloat)
    }
}

pub(super) async fn load(
    services: &ApplicationServices,
    capture: &catalog::Capture,
    lane: Lane,
    mut request: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
) -> Result<Rows, String> {
    let mut generation = capture.generation.clone();
    let cancelled = async {
        tokio::select! {
            _ = request.wait_for(|v| *v) => (),
            _ = generation.changed() => (),
            _ = capture.route.retired() => (),
            _ = stop.wait_for(|v| *v) => (),
        }
    };
    services
        .run_metadata(&capture.project, cancelled, async |client| {
            if !capture.route.is_current() || !matches!(capture.generation.has_changed(), Ok(false))
            {
                return Err(Error::ConnectionFailed("Connection changed".into()));
            }
            Ok(match lane {
                Lane::Stats => Rows::Stats(pg::load_database_stats(client).await?),
                Lane::Activity => Rows::Activity(typed(pg::load_activity(client).await?)?),
                Lane::Tables => Rows::Tables(typed(pg::load_table_stats(client).await?)?),
                Lane::Locks => Rows::locks(pg::load_active_locks(client).await?)?,
                Lane::Indexes => Rows::indexes(pg::load_index_usage(client).await?)?,
                Lane::Bloat => Rows::bloat(pg::load_table_bloat(client).await?)?,
            })
        })
        .await
        .map_err(|e| e.to_string())
}
