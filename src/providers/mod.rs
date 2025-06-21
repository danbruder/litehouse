pub mod cmd;
pub mod podman;

use crate::models::App;
use anyhow::Result;
use sqlx::{Pool, Sqlite};

pub trait Provider {
    type Handle: Handle;

    async fn start(&self, app: &App) -> Result<Self::Handle>;
    async fn setup(&self, _pool: &Pool<Sqlite>, app: &App) -> Result<App> {
        Ok(app.clone())
    }
    async fn teardown(&self, app: &App) -> Result<App> {
        Ok(app.clone())
    }
}

pub trait Handle {
    fn id(&self) -> u32;
    // fn is_running(&self) -> bool;
    // fn stop(&mut self) -> Result<()>;
    // fn restart(&mut self) -> Result<()>;
    //fn name(&self) -> String;
}

#[cfg(test)]
pub mod test {
    use super::*;

    pub struct TestProvider;

    impl Provider for TestProvider {
        type Handle = bool;

        async fn start(&self, _app: &App) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    impl Handle for bool {
        fn id(&self) -> u32 {
            1
        }
    }
}
