//! Maps an [`Engine`] to its [`Driver`] implementation.

use std::sync::Arc;

use crate::driver::Driver;
use crate::drivers;
use crate::types::Engine;

/// Holds one shared instance of each driver. Drivers are stateless (they
/// connect per call), so a single instance per engine is fine.
#[derive(Clone)]
pub struct Registry {
    mysql: Arc<dyn Driver>,
    redis: Arc<dyn Driver>,
    mongodb: Arc<dyn Driver>,
    clickhouse: Arc<dyn Driver>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            mysql: Arc::new(drivers::mysql::MysqlDriver::default()),
            redis: Arc::new(drivers::redis::RedisDriver::default()),
            mongodb: Arc::new(drivers::mongodb::MongoDriver::default()),
            clickhouse: Arc::new(drivers::clickhouse::ClickhouseDriver::default()),
        }
    }

    pub fn get(&self, engine: Engine) -> Arc<dyn Driver> {
        match engine {
            Engine::Mysql => Arc::clone(&self.mysql),
            Engine::Redis => Arc::clone(&self.redis),
            Engine::Mongodb => Arc::clone(&self.mongodb),
            Engine::Clickhouse => Arc::clone(&self.clickhouse),
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
