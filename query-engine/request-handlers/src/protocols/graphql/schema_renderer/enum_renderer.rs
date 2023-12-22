use super::*;

pub(crate) struct GqlEnumRenderer {
    enum_type: EnumType,
}

impl Renderer for GqlEnumRenderer {
    fn render(&self, ctx: &mut RenderContext) -> String {
        if ctx.already_rendered(&self.enum_type.name()) {
            return "".to_owned();
        }

        let values = self.format_enum_values();
        let values: Vec<String> = values
            .into_iter()
            .map(|(name, comment)| {
                if let Some(comment) = comment {
                    format!("\"\"\"{}\"\"\"\n{}", comment, name)
                } else {
                    name
                }
            })
            .collect();
        let rendered = format!("enum {} {{\n{}\n}}", self.enum_type.name(), values.join("\n"));

        ctx.add(self.enum_type.name(), rendered.clone());
        rendered
    }
}

impl GqlEnumRenderer {
    pub(crate) fn new(enum_type: EnumType) -> GqlEnumRenderer {
        GqlEnumRenderer { enum_type }
    }

    fn format_enum_values(&self) -> Vec<(String, Option<&str>)> {
        match &self.enum_type {
            EnumType::String(s) => s.values().to_owned().into_iter().map(|v| (v, None)).collect(),
            EnumType::Database(dbt) => dbt.external_values().into_iter().map(|v| (v, None)).collect(),
            EnumType::FieldRef(f) => f.values_with_comment(),
        }
    }
}
