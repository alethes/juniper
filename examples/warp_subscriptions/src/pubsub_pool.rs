// use mobc::async_trait;
// use mobc::Manager;
// pub use redis;
// pub use redis::aio::{Connection, PubSub};
// use redis::{Client};

// pub struct RedisPubSubManager {
//     client: Client,
// }

// impl RedisPubSubManager {
//     pub fn new(c: Client) -> Self {
//         Self { client: c }
//     }
// }

// #[async_trait]
// impl Manager for RedisPubSubManager {
//     type Connection = std::sync::Arc<std::sync::Mutex<PubSub>>;
//     type Error = redis::RedisError;

//     async fn connect(&self) -> Result<Self::Connection, Self::Error> {
//         let c = self.client.get_async_connection().await.unwrap();
//         let c = std::sync::Arc::new(c.into_pubsub());
//         Ok(c.clone())
//     }

//     async fn check(&self, mut conn: Self::Connection) -> Result<Self::Connection, Self::Error> {
//         // redis::cmd("PING").query_async(&mut conn).await?;
//         Ok(conn)
//     }
// }
