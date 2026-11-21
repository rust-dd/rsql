//! A pooled PostgreSQL client carries its own command-warning channel.

#[cfg(not(feature = "structure"))]
pub use deadpool_postgres::{Client, Pool};

#[cfg(feature = "structure")]
pub(super) use tracked::{CREATING, Manager};
#[cfg(feature = "structure")]
pub use tracked::{Client, Pool};

#[cfg(feature = "structure")]
mod tracked {
    use super::super::command_notices::NoticeSlot;
    use deadpool::managed;
    use std::ops::{Deref, DerefMut};

    pub type Pool = managed::Pool<Manager>;
    pub type Client = managed::Object<Manager>;
    #[derive(Debug)]
    pub struct Manager(pub(in crate::services) deadpool_postgres::Manager);

    // The manager and connector share a slot only while creating this physical connection.
    tokio::task_local! {
        pub(in crate::services) static CREATING: NoticeSlot;
    }

    #[derive(Debug)]
    pub struct Connection {
        inner: deadpool_postgres::ClientWrapper,
        pub(in crate::services) notices: NoticeSlot,
    }
    impl Deref for Connection {
        type Target = deadpool_postgres::ClientWrapper;
        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }
    impl DerefMut for Connection {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.inner
        }
    }
    impl managed::Manager for Manager {
        type Type = Connection;
        type Error = tokio_postgres::Error;
        async fn create(&self) -> Result<Connection, Self::Error> {
            let notices = NoticeSlot::default();
            let inner = CREATING.scope(notices.clone(), self.0.create()).await?;
            Ok(Connection { inner, notices })
        }
        async fn recycle(
            &self,
            client: &mut Connection,
            metrics: &managed::Metrics,
        ) -> managed::RecycleResult<Self::Error> {
            self.0.recycle(&mut client.inner, metrics).await
        }
        fn detach(&self, client: &mut Connection) {
            self.0.detach(&mut client.inner);
        }
    }
    impl Deref for Manager {
        type Target = deadpool_postgres::Manager;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
}
