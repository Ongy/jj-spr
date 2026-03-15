use graphql_client::GraphQLQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/request_reviews.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct RequestReviews;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/add_assignees.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct AddAssignees;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct UpdateIssueComment;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct AddComment;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct UpdatePRBase;
