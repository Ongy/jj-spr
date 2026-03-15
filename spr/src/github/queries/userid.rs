use graphql_client::GraphQLQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/add_assignees.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct UserId;
