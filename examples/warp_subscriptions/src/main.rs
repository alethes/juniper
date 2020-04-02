//! This example demonstrates asynchronous subscriptions with warp and tokio 0.2
#[macro_use]
extern crate lazy_static;
extern crate redis;

use std::{pin::Pin, sync::{Arc, Mutex}, time::Duration};
use futures::{Future, FutureExt as _, Stream, stream::StreamExt};
use juniper::{DefaultScalarValue, EmptyMutation, FieldError, RootNode};
use juniper_subscriptions::Coordinator;
use juniper_warp::{playground_filter, subscriptions::graphql_subscriptions};
use warp::{http::Response, Filter};
// use redis::Commands;
use std::collections::HashMap;
use std::sync::mpsc::{Sender};

lazy_static! {
    // static ref REDIS_CLIENT: redis::Client = redis::Client::open("redis://127.0.0.1/").unwrap();
    static ref SUB_MAP: SubManager = SubManager::new();
}

type SubStream = Pin<Box<dyn Stream<Item = ()> + Send>>;

struct A {
    pubsub: Arc<Mutex<redis::aio::PubSub>>,
    stream: SubStream
}

impl A {
    fn new(pubsub: Arc<Mutex<redis::aio::PubSub>>) -> A {
        let o = pubsub.to_owned();
        let m = o.lock().unwrap();
        A {
            pubsub,
            stream: Box::pin(m.on_message::<'static>().map(move |_m| {
                // let _ = pc;
                // let am = Arc::new(m);
                // match subs.get(&mchannel) {
                //     Some(subs) => {
                //         for s in subs {
                //             let amc = am.clone();
                //             s.send(amc);
                //         }
                //     },
                //     None => {

                //     }
                // }
            }))
        }
    }
}

struct SubManager {
    client: redis::Client,
    pubsubs: HashMap<String, Arc<Mutex<redis::aio::PubSub>>>,
    streams: HashMap<String, A>,
    stream_senders: HashMap<String, Sender<Sender<redis::Msg>>>
    // subscribers: HashMap<String, Vec<Sender<Arc<redis::Msg>>>>
}

// unsafe impl Sync for redis::aio::PubSub {}

impl SubManager {
    fn new() -> SubManager {
        let client = redis::Client::open("redis://127.0.0.1/").unwrap();
        SubManager {
            client,
            pubsubs: HashMap::new(),
            streams: HashMap::new(),
            stream_senders: HashMap::new()
            // subscribers: HashMap::new()
        }
    }
    async fn subscribe(&'static mut self, channel: String) {
        if let None = self.streams.get(&channel) {
            let pubsub = Arc::new(Mutex::new(async {
                let mut p = self.client.get_async_connection().await.unwrap().into_pubsub();
                p.subscribe(&channel).await;
                p
            }.await));
            self.pubsubs.insert(channel.clone(), pubsub.clone());
            let pc = pubsub.clone();
            let _mchannel = channel.clone();
            // let subs = self.subscribers;
            self.streams.insert(channel, A::new(pc));
        }
    }
    // async fn get_pubsub(&mut self, channel: String, mapper: fn(redis::Msg) -> Result<T, FieldError>) -> &SubStream {
    //     self.subscribe(channel.clone(), mapper);
    //     self.pubsubs.get(&channel).unwrap()
    // }
    // async fn get_stream(&'static mut self, channel: String, mapper: fn(redis::Msg) -> Result<T, FieldError>) -> SubStream {
    //     let stream = self.get_pubsub(channel, mapper);
    //     Box::pin(stream)
    // }
}

unsafe impl Sync for SubManager {}

#[derive(Clone)]
struct Context {}

impl juniper::Context for Context {}

#[derive(Clone, Copy, juniper::GraphQLEnum)]
enum UserKind {
    Admin,
    User,
    Guest,
}

struct User {
    id: i32,
    kind: UserKind,
    name: String,
}

// Field resolvers implementation
#[juniper::graphql_object(Context = Context)]
impl User {
    fn id(&self) -> i32 {
        self.id
    }

    fn kind(&self) -> UserKind {
        self.kind
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn friends(&self) -> Vec<User> {
        if self.id == 1 {
            return vec![
                User {
                    id: 11,
                    kind: UserKind::User,
                    name: "user11".into(),
                },
                User {
                    id: 12,
                    kind: UserKind::Admin,
                    name: "user12".into(),
                },
                User {
                    id: 13,
                    kind: UserKind::Guest,
                    name: "user13".into(),
                },
            ];
        } else if self.id == 2 {
            return vec![User {
                id: 21,
                kind: UserKind::User,
                name: "user21".into(),
            }];
        } else if self.id == 3 {
            return vec![
                User {
                    id: 31,
                    kind: UserKind::User,
                    name: "user31".into(),
                },
                User {
                    id: 32,
                    kind: UserKind::Guest,
                    name: "user32".into(),
                },
            ];
        } else {
            return vec![];
        }
    }
}

struct Query;

#[juniper::graphql_object(Context = Context)]
impl Query {
    async fn users(id: i32) -> Vec<User> {
        vec![User {
            id,
            kind: UserKind::Admin,
            name: "User Name".into(),
        }]
    }
}

struct Subscription;

#[juniper::graphql_subscription(Context = Context)]
impl Subscription {
    async fn users() -> Pin<Box<dyn Stream<Item = Result<User, FieldError>> + Send>> {
        let mut counter = 0;
        let stream = tokio::time::interval(Duration::from_secs(5)).map(move |_| {
            counter += 1;
            if counter == 2 {
                Err(FieldError::new(
                    "some field error from handler",
                    Value::Scalar(DefaultScalarValue::String(
                        "some additional string".to_string(),
                    )),
                ))
            } else {
                Ok(User {
                    id: counter,
                    kind: UserKind::Admin,
                    name: "stream user".to_string(),
                })
            }
        });

        Box::pin(stream)
    }
}

type Schema = RootNode<'static, Query, EmptyMutation<Context>, Subscription>;

fn schema() -> Schema {
    Schema::new(Query, EmptyMutation::new(), Subscription)
}

#[tokio::main]
async fn main() {
    ::std::env::set_var("RUST_LOG", "warp_subscriptions");
    env_logger::init();

    let log = warp::log("warp_server");

    let homepage = warp::path::end().map(|| {
        Response::builder()
            .header("content-type", "text/html")
            .body("<html><h1>juniper_subscriptions demo</h1><div>visit <a href=\"/playground\">graphql playground</a></html>".to_string())
    });

    let qm_schema = schema();
    let qm_state = warp::any().map(move || Context {});
    let qm_graphql_filter = juniper_warp::make_graphql_filter(qm_schema, qm_state.boxed());

    let sub_state = warp::any().map(move || Context {});
    let coordinator = Arc::new(juniper_subscriptions::Coordinator::new(schema()));

    log::info!("Listening on 127.0.0.1:8080");

    let routes = (warp::path("subscriptions")
        .and(warp::ws())
        .and(sub_state.clone())
        .and(warp::any().map(move || Arc::clone(&coordinator)))
        .map(
            |ws: warp::ws::Ws,
             ctx: Context,
             coordinator: Arc<Coordinator<'static, _, _, _, _, _>>| {
                ws.on_upgrade(|websocket| -> Pin<Box<dyn Future<Output = ()> + Send>> {
                    graphql_subscriptions(websocket, coordinator, ctx)
                        .map(|r| {
                            if let Err(e) = r {
                                println!("Websocket error: {}", e);
                            }
                        })
                        .boxed()
                })
            },
        ))
    .map(|reply| {
        // TODO#584: remove this workaround
        warp::reply::with_header(reply, "Sec-WebSocket-Protocol", "graphql-ws")
    })
    .or(warp::post()
        .and(warp::path("graphql"))
        .and(qm_graphql_filter))
    .or(warp::get()
        .and(warp::path("playground"))
        .and(playground_filter("/graphql", Some("/subscriptions"))))
    .or(homepage)
    .with(log);

    warp::serve(routes).run(([127, 0, 0, 1], 8080)).await;
}
