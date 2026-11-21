use rsql_core::ApplicationServices;
use rsql_desktop::connection::ConnectionProfile;
use std::pin::Pin;

pub type RouteResult<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

pub trait Routes: Send + Sync {
    fn trust<'a>(&'a self, id: &'a str) -> RouteResult<'a, ()>;
    fn request<'a>(&'a self, p: &'a ConnectionProfile, test: bool) -> RouteResult<'a, String>;
}

impl Routes for ApplicationServices {
    fn trust<'a>(&'a self, id: &'a str) -> RouteResult<'a, ()> {
        Box::pin(async move { self.trust_host_key(id).await.map_err(|e| e.to_string()) })
    }
    fn request<'a>(&'a self, p: &'a ConnectionProfile, test: bool) -> RouteResult<'a, String> {
        Box::pin(async move {
            if test {
                p.test(self).await
            } else {
                p.connect_saved(self).await.map(|_| "Connected".into())
            }
        })
    }
}
