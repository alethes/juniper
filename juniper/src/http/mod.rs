//! Utilities for building HTTP endpoints in a library-agnostic manner

pub mod graphiql;
pub mod playground;

#[cfg(feature = "async")]
use std::pin::Pin;

#[cfg(feature = "async")]
use futures::{stream::Stream, stream::StreamExt as _};
use serde::{
    de::Deserialize,
    ser::{self, Serialize, SerializeMap},
};
use serde_derive::{Deserialize, Serialize};

#[cfg(feature = "async")]
use crate::executor::ValuesResultStream;
use crate::{ast::InputValue, executor::ExecutionError, value, value::{DefaultScalarValue, ScalarValue}, FieldError, GraphQLError, GraphQLType, Object, RootNode, Value, Variables, SubscriptionConnection};
use futures::task::Poll;
use std::any::Any;

/// The expected structure of the decoded JSON document for either POST or GET requests.
///
/// For POST, you can use Serde to deserialize the incoming JSON data directly
/// into this struct - it derives Deserialize for exactly this reason.
///
/// For GET, you will need to parse the query string and extract "query",
/// "operationName", and "variables" manually.
#[derive(Deserialize, Clone, Serialize, PartialEq, Debug)]
pub struct GraphQLRequest<S = DefaultScalarValue>
where
    S: ScalarValue,
{
    query: String,
    #[serde(rename = "operationName")]
    operation_name: Option<String>,
    #[serde(bound(deserialize = "InputValue<S>: Deserialize<'de> + Serialize"))]
    variables: Option<InputValue<S>>,
}

impl<S> GraphQLRequest<S>
where
    S: ScalarValue,
{
    /// Returns the `operation_name` associated with this request.
    pub fn operation_name(&self) -> Option<&str> {
        self.operation_name.as_ref().map(|oper_name| &**oper_name)
    }

    fn variables(&self) -> Variables<S> {
        self.variables
            .as_ref()
            .and_then(|iv| {
                iv.to_object_value().map(|o| {
                    o.into_iter()
                        .map(|(k, v)| (k.to_owned(), v.clone()))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    /// Construct a new GraphQL request from parts
    pub fn new(
        query: String,
        operation_name: Option<String>,
        variables: Option<InputValue<S>>,
    ) -> Self {
        GraphQLRequest {
            query,
            operation_name,
            variables,
        }
    }

    pub async fn subscribe<'a, CtxT, CoordinatorT>(
        &'a self,
        coordinator: &'a CoordinatorT,
        context: &'a CtxT,
//    ) -> Result<Box<dyn SubscriptionConnection<'_, S> + 'a>, GraphQLError<'a>>
    ) -> Result<crate::Connection<'_, S>, GraphQLError<'a>>
        where
            S: ScalarValue + Send + Sync + 'static,
            CoordinatorT: crate::SubscriptionCoordinator<CtxT, S> + Send + Sync,
            CtxT: Send + Sync,
    {
        coordinator.subscribe(
            self,
            context
        ).await
    }

    /// Execute a GraphQL request using the specified schema and context
    ///
    /// This is a simple wrapper around the `execute` function exposed at the
    /// top level of this crate.
    pub fn execute<'a, CtxT, QueryT, MutationT, SubscriptionT>(
        &'a self,
        root_node: &'a RootNode<QueryT, MutationT, SubscriptionT, S>,
        context: &CtxT,
    ) -> GraphQLResponse<'a, S>
    where
        S: ScalarValue + Send + Sync + 'static,
        QueryT: GraphQLType<S, Context = CtxT>,
        MutationT: GraphQLType<S, Context = CtxT>,
        SubscriptionT: GraphQLType<S, Context = CtxT>,
    {
        GraphQLResponse(crate::execute(
            &self.query,
            self.operation_name(),
            root_node,
            &self.variables(),
            context,
        ))
    }

    /// Execute a GraphQL request asynchronously using the specified schema and context
    ///
    /// This is a simple wrapper around the `execute_async` function exposed at the
    /// top level of this crate.
    #[cfg(feature = "async")]
    pub async fn execute_async<'a, CtxT, QueryT, MutationT, SubscriptionT>(
        &'a self,
        root_node: &'a RootNode<'a, QueryT, MutationT, SubscriptionT, S>,
        context: &'a CtxT,
    ) -> GraphQLResponse<'a, S>
    where
        S: ScalarValue + Send + Sync + 'static,
        QueryT: crate::GraphQLTypeAsync<S, Context = CtxT> + Send + Sync,
        QueryT::TypeInfo: Send + Sync,
        MutationT: crate::GraphQLTypeAsync<S, Context = CtxT> + Send + Sync,
        MutationT::TypeInfo: Send + Sync,
        SubscriptionT: crate::GraphQLSubscriptionType<S, Context = CtxT> + Send + Sync,
        SubscriptionT::TypeInfo: Send + Sync,
        CtxT: Send + Sync,
    {
        let op = self.operation_name();
        let vars = &self.variables();
        let res = crate::execute_async(&self.query, op, root_node, vars, context).await;

        GraphQLResponse(res)
    }
}

/// Execute a GraphQL subscription using the specified schema and context
///
/// This is a wrapper around the `subscribe_async` function exposed
/// at the top level of this crate.
#[cfg(feature = "async")]
pub async fn resolve_into_stream<'req, 'rn, 'ctx, 'a, CtxT, QueryT, MutationT, SubscriptionT, S>(
    req: &'req GraphQLRequest<S>,
    root_node: &'rn RootNode<'a, QueryT, MutationT, SubscriptionT, S>,
    context: &'ctx CtxT,
) -> Result<
        (Value<ValuesResultStream<'a, S>>, Vec<ExecutionError<S>>),
        GraphQLError<'a>
    >
    where
        'req: 'a,
        'rn: 'a,
        'ctx: 'a,
        S: ScalarValue + Send + Sync + 'static,
    //todo: consider importing without 'crate::'
        QueryT: crate::GraphQLTypeAsync<S, Context = CtxT> + Send + Sync,
        QueryT::TypeInfo: Send + Sync,
        MutationT: crate::GraphQLTypeAsync<S, Context = CtxT> + Send + Sync,
        MutationT::TypeInfo: Send + Sync,
        SubscriptionT: crate::GraphQLSubscriptionType<S, Context = CtxT> + Send + Sync,
        SubscriptionT::TypeInfo: Send + Sync,
        CtxT: Send + Sync,
{
    let op = req.operation_name();
    let vars = req.variables();
    let res = crate::subscribe(
        &req.query,
        op,
        root_node,
        &vars,
        context
    ).await;

    //todo: return Connection::from(res) (?)
    res
}


/// Simple wrapper around the result from executing a GraphQL query
///
/// This struct implements Serialize, so you can simply serialize this
/// to JSON and send it over the wire. Use the `is_ok` method to determine
/// whether to send a 200 or 400 HTTP status code.
pub struct GraphQLResponse<'a, S = DefaultScalarValue>(
    Result<(Value<S>, Vec<ExecutionError<S>>), GraphQLError<'a>>,
);

impl<'a, S> GraphQLResponse<'a, S>
where
    S: ScalarValue,
{
    /// Constructs new `GraphQLResponse` using the given result
    pub fn from_result(r: Result<(Value<S>, Vec<ExecutionError<S>>), GraphQLError<'a>>) -> Self {
        Self(r)
    }

    /// Constructs an error response outside of the normal execution flow
    pub fn error(error: FieldError<S>) -> Self {
        GraphQLResponse(Ok((Value::null(), vec![ExecutionError::at_origin(error)])))
    }

    /// Was the request successful or not?
    ///
    /// Note that there still might be errors in the response even though it's
    /// considered OK. This is by design in GraphQL.
    pub fn is_ok(&self) -> bool {
        self.0.is_ok()
    }
}

impl<'a, T> Serialize for GraphQLResponse<'a, T>
where
    T: Serialize + ScalarValue,
    Value<T>: Serialize,
    ExecutionError<T>: Serialize,
    GraphQLError<'a>: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        match self.0 {
            Ok((ref res, ref err)) => {
                let mut map = serializer.serialize_map(None)?;

                map.serialize_key("data")?;
                map.serialize_value(res)?;

                if !err.is_empty() {
                    map.serialize_key("errors")?;
                    map.serialize_value(err)?;
                }

                map.end()
            }
            Err(ref err) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_key("errors")?;
                map.serialize_value(err)?;
                map.end()
            }
        }
    }
}

#[cfg(feature = "async")]
/// Wrapper around the asynchronous result from executing a GraphQL subscription
pub struct StreamGraphQLResponse<'a, S = DefaultScalarValue>(
    Result<(Value<ValuesResultStream<'a, S>>, Vec<ExecutionError<S>>), GraphQLError<'a>>,
)
where
    S: 'static;

#[cfg(feature = "async")]
impl<'a, S> StreamGraphQLResponse<'a, S>
where
    S: value::ScalarValue,
{
    /// Convert `StreamGraphQLResponse` to `Value<ValuesStream>`
    pub fn into_inner(
        self,
    ) -> Result<(Value<ValuesResultStream<'a, S>>, Vec<ExecutionError<S>>), GraphQLError<'a>> {
        self.0
    }
}

#[cfg(feature = "async")]
impl<'a, S> StreamGraphQLResponse<'a, S>
where
    S: value::ScalarValue + Send,
{
    /// Converts `Self` into default `Stream` implementantion
    pub fn into_stream(
        self,
    ) -> Result<
        Pin<Box<dyn futures::Stream<Item = GraphQLResponse<'static, S>> + Send + 'a>>,
        StreamError<'a, S>,
    > {
        use std::iter::FromIterator as _;

        let val = match self.0 {
            Ok((v, err_vec)) => match err_vec.len() {
                0 => v,
                _ => return Err(StreamError::Execution(err_vec)),
            },
            Err(e) => return Err(StreamError::GraphQL(e)),
        };

        match val {
            Value::Null => Err(StreamError::NullValue),
            Value::Scalar(stream) => {
                Ok(Box::pin(stream.map(|value| {
                    match value {
                        Ok(val) => GraphQLResponse::from_result(Ok((val, vec![]))),
                        // TODO#433: not return random error
                        Err(_) => GraphQLResponse::from_result(Err(GraphQLError::IsSubscription)),
                    }
                })))
            }
            // TODO#433: remove this implementation and add a // TODO: implement these
            //           (current implementation might be confusing)
            Value::List(_) => return Err(StreamError::ListValue),
            Value::Object(obj) => {
                let mut key_values = obj.into_key_value_list();
                if key_values.is_empty() {
                    return Err(StreamError::EmptyObject);
                }

                let mut filled_count = 0;
                let mut ready_vector = Vec::with_capacity(key_values.len());
                for _ in 0..key_values.len() {
                    ready_vector.push(None);
                }

                let stream = futures::stream::poll_fn(
                    move |mut ctx| -> Poll<Option<GraphQLResponse<'static, S>>> {
                        for i in 0..ready_vector.len() {
                            let val = &mut ready_vector[i];
                            if val.is_none() {
                                let (field_name, ref mut stream_val) = &mut key_values[i];

                                match stream_val {
                                Value::Scalar(stream) => {
                                    match Pin::new(stream).poll_next(&mut ctx) {
                                        Poll::Ready(None) => {
                                            return Poll::Ready(None);
                                        },
                                        Poll::Ready(Some(value)) => {
                                            *val = Some((field_name.clone(), value));
                                            filled_count += 1;
                                        }
                                        Poll::Pending => { /* check back later */ }
                                    }
                                },
                                    // TODO#433: not panic on errors
                                    _ => panic!("into_stream supports only Value::Scalar returned in Value::Object")
                            }
                            }
                        }
                        if filled_count == key_values.len() {
                            filled_count = 0;
                            let new_vec = (0..key_values.len()).map(|_| None).collect::<Vec<_>>();
                            let ready_vec = std::mem::replace(&mut ready_vector, new_vec);
                            let ready_vec_iterator = ready_vec.into_iter().map(|el| {
                                let (name, val) = el.unwrap();
                                if let Ok(value) = val {
                                    (name, value)
                                } else {
                                    (name, Value::Null)
                                }
                            });
                            let obj = Object::from_iter(ready_vec_iterator);
                            return Poll::Ready(Some(GraphQLResponse::from_result(Ok((
                                Value::Object(obj),
                                vec![],
                            )))));
                        } else {
                            return Poll::Pending;
                        }
                    },
                );

                Ok(Box::pin(stream))
            }
        }
    }
}

//TODO#433: consider re-looking at fields after
//          StreamGraphQLResponse::into_stream was re-implemented
/// Errors that can be returned from `StreamGraphQLResponse`
#[derive(Serialize, Debug)]
#[serde(untagged)]
pub enum StreamError<'a, S>
where
    S: crate::ScalarValue,
{
    /// ExecutionError
    Execution(Vec<ExecutionError<S>>),

    /// GraphQLError
    GraphQL(GraphQLError<'a>),

    /// If got Value::Null
    NullValue,

    /// If got Value::List. It is here because
    /// into_stream is not yet implemented for Value::List
    ListValue,

    /// If got Value::Object with length of 0
    EmptyObject,
}

#[cfg(any(test, feature = "expose-test-schema"))]
#[allow(missing_docs)]
pub mod tests {
    use serde_json::{self, Value as Json};

    /// Normalized response content we expect to get back from
    /// the http framework integration we are testing.
    #[derive(Debug)]
    pub struct TestResponse {
        pub status_code: i32,
        pub body: Option<String>,
        pub content_type: String,
    }

    /// Normalized way to make requests to the http framework
    /// integration we are testing.
    pub trait HTTPIntegration {
        fn get(&self, url: &str) -> TestResponse;
        fn post(&self, url: &str, body: &str) -> TestResponse;
    }

    #[allow(missing_docs)]
    pub fn run_http_test_suite<T: HTTPIntegration>(integration: &T) {
        println!("Running HTTP Test suite for integration");

        println!("  - test_simple_get");
        test_simple_get(integration);

        println!("  - test_encoded_get");
        test_encoded_get(integration);

        println!("  - test_get_with_variables");
        test_get_with_variables(integration);

        println!("  - test_simple_post");
        test_simple_post(integration);

        println!("  - test_batched_post");
        test_batched_post(integration);

        println!("  - test_invalid_json");
        test_invalid_json(integration);

        println!("  - test_invalid_field");
        test_invalid_field(integration);

        println!("  - test_duplicate_keys");
        test_duplicate_keys(integration);
    }

    fn unwrap_json_response(response: &TestResponse) -> Json {
        serde_json::from_str::<Json>(
            response
                .body
                .as_ref()
                .expect("No data returned from request"),
        )
        .expect("Could not parse JSON object")
    }

    fn test_simple_get<T: HTTPIntegration>(integration: &T) {
        // {hero{name}}
        let response = integration.get("/?query=%7Bhero%7Bname%7D%7D");

        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_type.as_str(), "application/json");

        assert_eq!(
            unwrap_json_response(&response),
            serde_json::from_str::<Json>(r#"{"data": {"hero": {"name": "R2-D2"}}}"#)
                .expect("Invalid JSON constant in test")
        );
    }

    fn test_encoded_get<T: HTTPIntegration>(integration: &T) {
        // query { human(id: "1000") { id, name, appearsIn, homePlanet } }
        let response = integration.get(
            "/?query=query%20%7B%20human(id%3A%20%221000%22)%20%7B%20id%2C%20name%2C%20appearsIn%2C%20homePlanet%20%7D%20%7D");

        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_type.as_str(), "application/json");

        assert_eq!(
            unwrap_json_response(&response),
            serde_json::from_str::<Json>(
                r#"{
                    "data": {
                        "human": {
                            "appearsIn": [
                                "NEW_HOPE",
                                "EMPIRE",
                                "JEDI"
                                ],
                                "homePlanet": "Tatooine",
                                "name": "Luke Skywalker",
                                "id": "1000"
                            }
                        }
                    }"#
            )
            .expect("Invalid JSON constant in test")
        );
    }

    fn test_get_with_variables<T: HTTPIntegration>(integration: &T) {
        // query($id: String!) { human(id: $id) { id, name, appearsIn, homePlanet } }
        // with variables = { "id": "1000" }
        let response = integration.get(
            "/?query=query(%24id%3A%20String!)%20%7B%20human(id%3A%20%24id)%20%7B%20id%2C%20name%2C%20appearsIn%2C%20homePlanet%20%7D%20%7D&variables=%7B%20%22id%22%3A%20%221000%22%20%7D");

        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_type, "application/json");

        assert_eq!(
            unwrap_json_response(&response),
            serde_json::from_str::<Json>(
                r#"{
                    "data": {
                        "human": {
                            "appearsIn": [
                                "NEW_HOPE",
                                "EMPIRE",
                                "JEDI"
                                ],
                                "homePlanet": "Tatooine",
                                "name": "Luke Skywalker",
                                "id": "1000"
                            }
                        }
                    }"#
            )
            .expect("Invalid JSON constant in test")
        );
    }

    fn test_simple_post<T: HTTPIntegration>(integration: &T) {
        let response = integration.post("/", r#"{"query": "{hero{name}}"}"#);

        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_type, "application/json");

        assert_eq!(
            unwrap_json_response(&response),
            serde_json::from_str::<Json>(r#"{"data": {"hero": {"name": "R2-D2"}}}"#)
                .expect("Invalid JSON constant in test")
        );
    }

    fn test_batched_post<T: HTTPIntegration>(integration: &T) {
        let response = integration.post(
            "/",
            r#"[{"query": "{hero{name}}"}, {"query": "{hero{name}}"}]"#,
        );

        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_type, "application/json");

        assert_eq!(
            unwrap_json_response(&response),
            serde_json::from_str::<Json>(
                r#"[{"data": {"hero": {"name": "R2-D2"}}}, {"data": {"hero": {"name": "R2-D2"}}}]"#
            )
            .expect("Invalid JSON constant in test")
        );
    }

    fn test_invalid_json<T: HTTPIntegration>(integration: &T) {
        let response = integration.get("/?query=blah");
        assert_eq!(response.status_code, 400);
        let response = integration.post("/", r#"blah"#);
        assert_eq!(response.status_code, 400);
    }

    fn test_invalid_field<T: HTTPIntegration>(integration: &T) {
        // {hero{blah}}
        let response = integration.get("/?query=%7Bhero%7Bblah%7D%7D");
        assert_eq!(response.status_code, 400);
        let response = integration.post("/", r#"{"query": "{hero{blah}}"}"#);
        assert_eq!(response.status_code, 400);
    }

    fn test_duplicate_keys<T: HTTPIntegration>(integration: &T) {
        // {hero{name}}
        let response = integration.get("/?query=%7B%22query%22%3A%20%22%7Bhero%7Bname%7D%7D%22%2C%20%22query%22%3A%20%22%7Bhero%7Bname%7D%7D%22%7D");
        assert_eq!(response.status_code, 400);
        let response = integration.post(
            "/",
            r#"
            {"query": "{hero{name}}", "query": "{hero{name}}"}
        "#,
        );
        assert_eq!(response.status_code, 400);
    }
}
