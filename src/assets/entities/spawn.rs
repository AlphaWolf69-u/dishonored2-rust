//! Authored component slot identity shared by declaration processing and admission.
/// Component slot identity shared by authored resources and the resolved batch.
/// Returns None for non-slot edit-list entries such as item_begin.
pub fn component_slot_identity(key: &str) -> Result<Option<String>, String> {
    let (numeric, suffix) = if let Some(value) = key.strip_prefix("item_add[") {
        (false, value)
    } else if let Some(value) = key.strip_prefix("item_override[") {
        (false, value)
    } else if let Some(value) = key.strip_prefix("item[") {
        (true, value)
    } else { return Ok(None) };
    let slot = suffix.strip_suffix(']').ok_or_else(|| format!("invalid component slot {key}"))?;
    if numeric {
        if slot.starts_with('"') { return Err(format!("quoted numeric component slot requires native handling: {key}")); }
        let index = slot.parse::<i32>().map_err(|_| format!("invalid numeric component slot {slot}"))?;
        Ok(Some(super::processed_decl::encode_edit_id(index.wrapping_add(1) as u32)))
    } else if slot.starts_with('"') {
        let bytes = super::processed_decl::decode_quoted(slot)?;
        Ok(Some(String::from_utf8(bytes).map_err(|_| format!("non-UTF-8 component slot {key}"))?))
    } else { Ok(Some(slot.to_owned())) }
}
