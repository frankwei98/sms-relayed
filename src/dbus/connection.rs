use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use zbus::Connection;

#[derive(Clone, Default)]
pub(super) struct SystemConnectionCache {
    connection: Arc<Mutex<Option<Arc<Connection>>>>,
}

impl SystemConnectionCache {
    pub(super) async fn connect() -> Result<Self> {
        let connection = Connection::system().await?;
        Ok(Self {
            connection: Arc::new(Mutex::new(Some(Arc::new(connection)))),
        })
    }

    pub(super) async fn get_or_connect(&self) -> Result<Arc<Connection>> {
        let mut connection = self.connection.lock().await;
        if let Some(connection) = connection.as_ref() {
            return Ok(connection.clone());
        }
        let new_connection = Arc::new(Connection::system().await?);
        *connection = Some(new_connection.clone());
        Ok(new_connection)
    }

    pub(super) async fn discard_if_current(&self, failed: &Arc<Connection>) {
        let mut connection = self.connection.lock().await;
        if connection
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, failed))
        {
            *connection = None;
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.connection.try_lock().unwrap().is_none()
    }

    #[cfg(test)]
    pub(super) fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.connection, &other.connection)
    }
}

#[cfg(test)]
mod tests {
    use super::SystemConnectionCache;

    #[test]
    fn default_cache_is_empty_without_connecting() {
        let cache = SystemConnectionCache::default();

        assert!(cache.is_empty());
    }

    #[test]
    fn clones_share_the_same_connection_state() {
        let cache = SystemConnectionCache::default();
        let cloned = cache.clone();

        assert!(cache.shares_state_with(&cloned));
    }
}
