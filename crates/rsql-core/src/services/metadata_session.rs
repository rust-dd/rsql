use super::{
    pool_connection::{checked_out, project_pool},
    query_session::settle_cancelled,
};
use crate::services::pooled::Client;
use crate::{ApplicationServices, Error};

impl ApplicationServices {
    /// Run one cancellable metadata operation on the project's metadata pool.
    /// The caller signals cancellation and continues driving this future until it returns.
    pub async fn run_metadata<T>(
        &self,
        project: &str,
        cancelled: impl Future<Output = ()>,
        operation: impl AsyncFnOnce(&tokio_postgres::Client) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let (pool, ssl, mut route) = {
            let _guard = self.lifecycle.lock().await;
            if *self.stopped.borrow() || !self.tickets.lock().unwrap().contains_key(project) {
                return Err(super::closed());
            }
            let pool = project_pool(&self.meta_clients, project).await?;
            let (_, ssl) = self.published_route(project).await?;
            let route = self.register_activity(project).await;
            (pool, ssl, route)
        };
        let stop = async {
            tokio::select! { _ = cancelled => (), _ = route.changed() => () }
        };
        tokio::pin!(stop);
        let client = tokio::select! { biased;
            _ = &mut stop => return Err(super::closed()),
            client = checked_out(&pool) => client?,
        };
        let result = tokio::select! { biased;
            _ = &mut stop => Err(super::closed()),
            result = operation(&client) => result,
        };
        if result.is_err() {
            // A cancelled or timed-out response may still be executing. Never return
            // this checkout to the pool, where a late CancelRequest could hit another user.
            settle_cancelled(&client, ssl).await;
            drop(Client::take(client));
        }
        result
    }
}
