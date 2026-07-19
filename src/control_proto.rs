include!(concat!(env!("OUT_DIR"), "/waywallen.control.v1.rs"));

use crate::plugin::renderer_registry::{SettingDef, SettingType};

/// Stringify a `toml::Value` for the wire `default_value` / `min` /
/// `max` / `step` fields.
fn toml_value_to_wire(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        // Arrays/tables aren't valid setting scalars; fall back to the
        // TOML repr so the UI sees a deterministic fallback string.
        other => other.to_string(),
    }
}

fn setting_type_to_proto(ty: SettingType) -> i32 {
    match ty {
        SettingType::U32 => SettingValueType::U32 as i32,
        SettingType::I32 => SettingValueType::I32 as i32,
        SettingType::F32 => SettingValueType::F32 as i32,
        SettingType::String => SettingValueType::String as i32,
        SettingType::Bool => SettingValueType::Bool as i32,
    }
}

/// Convert a source plugin's Lua-declared `SourceSetting` into the same
/// `SettingSchema` wire shape, so `SourcePluginInfo.settings` renders through
/// the identical UI as renderer settings.
pub fn source_setting_to_proto(s: &crate::plugin::source_manager::SourceSetting) -> SettingSchema {
    let ty = match s.ty.as_str() {
        "u32" => SettingValueType::U32,
        "i32" => SettingValueType::I32,
        "f32" => SettingValueType::F32,
        "bool" => SettingValueType::Bool,
        _ => SettingValueType::String,
    };
    SettingSchema {
        key: s.key.clone(),
        r#type: ty as i32,
        default_value: s.default.clone(),
        identity: false,
        label_key: s.label.clone(),
        description_key: s.description.clone(),
        min: String::new(),
        max: String::new(),
        step: String::new(),
        choices: s.choices.clone(),
        group: s.group.clone(),
        order: s.order,
    }
}

pub fn source_action_to_proto(a: &crate::plugin::source_manager::SourceAction) -> PluginActionDef {
    PluginActionDef {
        id: a.id.clone(),
        label: a.label.clone(),
        group: a.group.clone(),
        order: a.order,
    }
}

pub fn source_status_to_proto(s: &crate::plugin::source_manager::SourceStatus) -> PluginStatusRow {
    PluginStatusRow {
        id: s.id.clone(),
        label: s.label.clone(),
        group: s.group.clone(),
        order: s.order,
        value: s.value.clone(),
    }
}

/// Convert one manifest `SettingDef` into the `SettingSchema` wire
/// shape consumed by `RendererPluginInfo.settings`.
pub fn setting_def_to_proto(key: &str, def: &SettingDef) -> SettingSchema {
    SettingSchema {
        key: key.to_string(),
        r#type: setting_type_to_proto(def.ty),
        default_value: toml_value_to_wire(&def.default),
        identity: def.identity,
        label_key: def.label_key.clone().unwrap_or_default(),
        description_key: def.description_key.clone().unwrap_or_default(),
        min: def.min.as_ref().map(toml_value_to_wire).unwrap_or_default(),
        max: def.max.as_ref().map(toml_value_to_wire).unwrap_or_default(),
        step: def
            .step
            .as_ref()
            .map(toml_value_to_wire)
            .unwrap_or_default(),
        choices: def.choices.clone().unwrap_or_default(),
        group: def.group.clone().unwrap_or_default(),
        order: def.order.unwrap_or(0),
    }
}
