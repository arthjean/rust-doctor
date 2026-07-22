use rust_doctor::diagnostics::ReportV1;

fn main() {
    let Ok(mut schema) = serde_json::to_value(schemars::schema_for!(ReportV1)) else {
        eprintln!("failed to serialize the generated Report V1 schema");
        std::process::exit(1);
    };
    let Some(object) = schema.as_object_mut() else {
        eprintln!("generated Report V1 schema is not an object");
        std::process::exit(1);
    };
    object.insert(
        "$id".to_string(),
        serde_json::Value::String(
            "https://rust-doctor.vercel.app/schemas/report-v1.schema.json".to_string(),
        ),
    );
    let Ok(json) = serde_json::to_string_pretty(&schema) else {
        eprintln!("failed to format the generated Report V1 schema");
        std::process::exit(1);
    };
    println!("{json}");
}
