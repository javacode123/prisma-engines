use super::*;
use input_types::fields::arguments;
use mutations::{create_many, create_one};
use psl::datamodel_connector::ConnectorCapability;
use query_structure::{DefaultKind, PrismaValue};

/// Builds the root `Mutation` type.
pub(crate) fn mutation_fields(ctx: &QuerySchema) -> Vec<FieldFn> {
    let mut fields: Vec<FieldFn> = Vec::with_capacity(ctx.internal_data_model.schema.db.models_count() * 2);

    macro_rules! field {
        ($f:ident, $model_var:expr) => {{
            let model = $model_var.clone();
            fields.push(Box::new(move |ctx| $f(ctx, model.clone())));
        }};
    }

    for model in ctx.internal_data_model.models() {
        if model.supports_create_operation() {
            field!(create_one, model);

            field!(upsert_item_field, model);
            if ctx.has_capability(ConnectorCapability::CreateMany) {
                field!(create_many, model);
            }
        }

        field!(delete_item_field, model);
        field!(update_item_field, model);

        field!(update_many_field, model);
        field!(batch_update_many_field, model);
        field!(delete_many_field, model);
    }

    if ctx.enable_raw_queries && ctx.has_capability(ConnectorCapability::SqlQueryRaw) {
        fields.push(Box::new(|_| create_execute_raw_field()));
        fields.push(Box::new(|_| create_query_raw_field()));
    }

    if ctx.enable_raw_queries && ctx.has_capability(ConnectorCapability::MongoDbQueryRaw) {
        fields.push(Box::new(|_| create_mongodb_run_command_raw()));
    }

    fields
}

fn create_execute_raw_field<'a>() -> OutputField<'a> {
    field(
        "executeRaw",
        || {
            vec![
                input_field("query", vec![InputType::string()], None, None),
                input_field(
                    "parameters",
                    vec![InputType::json_list()],
                    Some(DefaultKind::Single(PrismaValue::String("[]".into()))),
                    None,
                )
                .optional(),
            ]
        },
        OutputType::non_list(OutputType::json()),
        Some(QueryInfo {
            tag: QueryTag::ExecuteRaw,
            model: None,
        }),
        None,
    )
}

fn create_query_raw_field<'a>() -> OutputField<'a> {
    field(
        "queryRaw",
        || {
            vec![
                simple_input_field("query", InputType::string(), None, None),
                simple_input_field(
                    "parameters",
                    InputType::json_list(),
                    Some(DefaultKind::Single(PrismaValue::String("[]".into()))),
                    None,
                )
                .optional(),
            ]
        },
        OutputType::non_list(OutputType::json()),
        Some(QueryInfo {
            tag: QueryTag::QueryRaw,
            model: None,
        }),
        None,
    )
}

fn create_mongodb_run_command_raw<'a>() -> OutputField<'a> {
    field(
        "runCommandRaw",
        || vec![simple_input_field("command", InputType::json(), None, None)],
        OutputType::non_list(OutputType::json()),
        Some(QueryInfo {
            tag: QueryTag::RunCommandRaw,
            model: None,
        }),
        None,
    )
}

/// Builds a delete mutation field (e.g. deleteUser) for given model.
fn delete_item_field(ctx: &QuerySchema, model: Model) -> OutputField<'_> {
    let cloned_model = model.clone();
    let model_id = model.id;
    field(
        format!("deleteOne{}", model.name()),
        move || arguments::delete_one_arguments(ctx, cloned_model),
        OutputType::object(objects::model::model_object_type(ctx, model)),
        Some(QueryInfo {
            model: Some(model_id),
            tag: QueryTag::DeleteOne,
        }),
        None,
    )
    .nullable()
}

/// Builds a delete many mutation field (e.g. deleteManyUsers) for given model.
fn delete_many_field(ctx: &QuerySchema, model: Model) -> OutputField<'_> {
    let field_name = format!("deleteMany{}", model.name());
    let cloned_model = model.clone();

    field(
        field_name,
        move || arguments::delete_many_arguments(ctx, cloned_model),
        OutputType::object(objects::affected_records_object_type()),
        Some(QueryInfo {
            model: Some(model.id),
            tag: QueryTag::DeleteMany,
        }),
        None,
    )
}

/// Builds an update mutation field (e.g. updateUser) for given model.
fn update_item_field(ctx: &QuerySchema, model: Model) -> OutputField<'_> {
    let field_name = format!("updateOne{}", model.name());
    let model_id = model.id;
    let cloned_model = model.clone();
    field(
        field_name,
        move || arguments::update_one_arguments(ctx, model),
        OutputType::object(objects::model::model_object_type(ctx, cloned_model)),
        Some(QueryInfo {
            model: Some(model_id),
            tag: QueryTag::UpdateOne,
        }),
        None,
    )
    .nullable()
}

/// Builds an update many mutation field (e.g. updateManyUsers) for given model.
fn update_many_field(ctx: &QuerySchema, model: Model) -> OutputField<'_> {
    let field_name = format!("updateMany{}", model.name());
    let cloned_model = model.clone();

    field(
        field_name,
        move || arguments::update_many_arguments(ctx, cloned_model),
        OutputType::object(objects::affected_records_object_type()),
        Some(QueryInfo {
            model: Some(model.id),
            tag: QueryTag::UpdateMany,
        }),
        None,
    )
}

/// Builds an update batch many mutation field (e.g. batchUpdateManyUsers) for given model.
fn batch_update_many_field(ctx: &QuerySchema, model: Model) -> OutputField<'_> {
    let field_name = format!("batchUpdateMany{}", model.name());
    let cloned_model = model.clone();

    field(
        field_name,
        move || arguments::batch_update_many_arguments(ctx, cloned_model),
        OutputType::object(objects::affected_records_object_type()),
        Some(QueryInfo {
            model: Some(model.id),
            tag: QueryTag::BatchUpdateMany,
        }),
        None,
    )
}

/// Builds an upsert mutation field (e.g. upsertUser) for given model.
fn upsert_item_field(ctx: &QuerySchema, model: Model) -> OutputField<'_> {
    let cloned_model = model.clone();
    let model_id = model.id;
    field(
        format!("upsertOne{}", model.name()),
        move || arguments::upsert_arguments(ctx, model),
        OutputType::object(objects::model::model_object_type(ctx, cloned_model)),
        Some(QueryInfo {
            model: Some(model_id),
            tag: QueryTag::UpsertOne,
        }),
        None,
    )
}
