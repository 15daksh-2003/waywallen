use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{
    sea_query::{Expr, LikeExpr},
    Condition,
};

use crate::control_proto as pb;

use super::entities::{item, library};

pub fn build_grouped_condition<'a, F, GFn, CFn>(
    filters: impl IntoIterator<Item = &'a F>,
    group_of: GFn,
    to_condition: CFn,
    logics: &[pb::FilterLogic],
) -> Option<Condition>
where
    F: 'a,
    GFn: Fn(&F) -> i32,
    CFn: Fn(&F) -> Option<Condition>,
{
    let mut buckets: BTreeMap<i32, Vec<Condition>> = BTreeMap::new();
    for filter in filters {
        if let Some(condition) = to_condition(filter) {
            buckets.entry(group_of(filter)).or_default().push(condition);
        }
    }
    if buckets.is_empty() {
        return None;
    }

    let group_cond = |group: i32| -> Option<Condition> {
        buckets.get(&group).map(|conditions| {
            let mut out = Condition::all();
            for condition in conditions {
                out = out.add(condition.clone());
            }
            out
        })
    };

    let mut pair_conds = Vec::new();
    let mut referenced = BTreeSet::<i32>::new();
    for logic in logics {
        let a = group_cond(logic.group_a);
        let b = group_cond(logic.group_b);
        let (a, b) = match (a, b) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };
        let combined = match pb::LogicOp::try_from(logic.op).unwrap_or(pb::LogicOp::And) {
            pb::LogicOp::Or => Condition::any().add(a).add(b),
            _ => Condition::all().add(a).add(b),
        };
        pair_conds.push(combined);
        referenced.insert(logic.group_a);
        referenced.insert(logic.group_b);
    }

    let mut outer = Condition::all();
    for pair in pair_conds {
        outer = outer.add(pair);
    }
    for group in buckets.keys() {
        if !referenced.contains(group) {
            if let Some(condition) = group_cond(*group) {
                outer = outer.add(condition);
            }
        }
    }
    Some(outer)
}

pub fn wallpaper_filters_to_condition(
    filters: &[pb::WallpaperFilterRule],
    logics: &[pb::FilterLogic],
) -> Option<Condition> {
    build_grouped_condition(
        filters,
        |filter| filter.group,
        wallpaper_filter_to_condition,
        logics,
    )
}

pub fn wallpaper_query_to_condition(
    filters: &[pb::WallpaperFilterRule],
    logics: &[pb::FilterLogic],
    search_text: &str,
) -> Option<Condition> {
    match (
        wallpaper_filters_to_condition(filters, logics),
        wallpaper_search_to_condition(search_text),
    ) {
        (Some(filters), Some(search)) => Some(Condition::all().add(filters).add(search)),
        (Some(filters), None) => Some(filters),
        (None, Some(search)) => Some(search),
        (None, None) => None,
    }
}

fn wallpaper_search_to_condition(search_text: &str) -> Option<Condition> {
    let search_text = search_text.trim();
    if search_text.is_empty() {
        return None;
    }

    let text = text_contains_condition(search_text)?;
    let pattern = literal_contains_pattern(search_text);
    let external_id = Expr::col((item::Entity, item::Column::ExternalId))
        .like(LikeExpr::new(pattern).escape('\\'));
    Some(Condition::any().add(text).add(external_id))
}

pub fn wallpaper_filter_to_condition(filter: &pb::WallpaperFilterRule) -> Option<Condition> {
    use pb::wallpaper_filter_rule::Payload;

    match pb::WallpaperFilterType::try_from(filter.r#type).ok()? {
        pb::WallpaperFilterType::Name => match filter.payload.as_ref() {
            Some(Payload::StringFilter(f)) => name_condition_to_condition(f),
            _ => None,
        },
        pb::WallpaperFilterType::WpType => match filter.payload.as_ref() {
            Some(Payload::StringFilter(f)) => string_condition_to_condition(
                || Expr::col((item::Entity, item::Column::Ty)),
                pb::StringCondition::try_from(f.condition)
                    .unwrap_or(pb::StringCondition::Unspecified),
                &f.value.to_ascii_lowercase(),
                false,
            ),
            _ => None,
        },
        pb::WallpaperFilterType::Library => match filter.payload.as_ref() {
            Some(Payload::StringFilter(f)) => string_condition_to_condition(
                || Expr::col((library::Entity, library::Column::Path)),
                pb::StringCondition::try_from(f.condition)
                    .unwrap_or(pb::StringCondition::Unspecified),
                &f.value,
                false,
            ),
            _ => None,
        },
        pb::WallpaperFilterType::Width => match filter.payload.as_ref() {
            Some(Payload::IntFilter(f)) => int_condition_to_condition(
                || Expr::col((item::Entity, item::Column::Width)),
                pb::IntCondition::try_from(f.condition).unwrap_or(pb::IntCondition::Unspecified),
                f.value,
            ),
            _ => None,
        },
        pb::WallpaperFilterType::Height => match filter.payload.as_ref() {
            Some(Payload::IntFilter(f)) => int_condition_to_condition(
                || Expr::col((item::Entity, item::Column::Height)),
                pb::IntCondition::try_from(f.condition).unwrap_or(pb::IntCondition::Unspecified),
                f.value,
            ),
            _ => None,
        },
        pb::WallpaperFilterType::Size => match filter.payload.as_ref() {
            Some(Payload::IntFilter(f)) => int_condition_to_condition(
                || Expr::col((item::Entity, item::Column::Size)),
                pb::IntCondition::try_from(f.condition).unwrap_or(pb::IntCondition::Unspecified),
                f.value,
            ),
            _ => None,
        },
        pb::WallpaperFilterType::ContentRating => match filter.payload.as_ref() {
            Some(Payload::StringFilter(f)) => string_condition_to_condition(
                || Expr::col((item::Entity, item::Column::ContentRating)),
                pb::StringCondition::try_from(f.condition)
                    .unwrap_or(pb::StringCondition::Unspecified),
                &f.value,
                true,
            ),
            _ => None,
        },
        pb::WallpaperFilterType::Tag => match filter.payload.as_ref() {
            Some(Payload::TagFilter(f)) => tag_list_condition_to_condition(
                &f.values,
                pb::StringCondition::try_from(f.condition)
                    .unwrap_or(pb::StringCondition::Unspecified),
            ),
            // Back-compat: older rules carried a single tag in string_filter.
            Some(Payload::StringFilter(f)) => tag_condition_to_condition(
                &f.value,
                pb::StringCondition::try_from(f.condition)
                    .unwrap_or(pb::StringCondition::Unspecified),
            ),
            _ => None,
        },
        pb::WallpaperFilterType::Unspecified => None,
    }
}

/// Tag membership predicate. SQLite COLLATE NOCASE handles case
/// insensitivity for tag names. CONTAINS / CONTAINS_NOT are nonsensical
fn tag_condition_to_condition(tag: &str, cond: pb::StringCondition) -> Option<Condition> {
    let tag_quoted = sqlite_quote(tag);
    let exists = format!(
        "EXISTS (SELECT 1 FROM item_tag JOIN tag ON tag.id = item_tag.tag_id \
         WHERE item_tag.item_id = item.id AND tag.name = {tag_quoted} COLLATE NOCASE)"
    );
    match cond {
        pb::StringCondition::Is | pb::StringCondition::Contains => {
            Some(Condition::all().add(Expr::cust(exists)))
        }
        pb::StringCondition::IsNot | pb::StringCondition::ContainsNot => {
            Some(Condition::all().add(Expr::cust(format!("NOT {exists}"))))
        }
        pb::StringCondition::Unspecified => None,
    }
}

/// Tag-set membership. IS → has any of `values`; IS_NOT → has none of
/// them. Empty list imposes no constraint.
fn tag_list_condition_to_condition(
    values: &[String],
    cond: pb::StringCondition,
) -> Option<Condition> {
    let names: Vec<&String> = values.iter().filter(|v| !v.is_empty()).collect();
    if names.is_empty() {
        return None;
    }
    let exists = |tag: &str| {
        format!(
            "EXISTS (SELECT 1 FROM item_tag JOIN tag ON tag.id = item_tag.tag_id \
             WHERE item_tag.item_id = item.id AND tag.name = {} COLLATE NOCASE)",
            sqlite_quote(tag)
        )
    };
    match cond {
        pb::StringCondition::Is | pb::StringCondition::Contains => {
            let mut any = Condition::any();
            for tag in names {
                any = any.add(Expr::cust(exists(tag)));
            }
            Some(any)
        }
        pb::StringCondition::IsNot | pb::StringCondition::ContainsNot => {
            let mut all = Condition::all();
            for tag in names {
                all = all.add(Expr::cust(format!("NOT {}", exists(tag))));
            }
            Some(all)
        }
        pb::StringCondition::Unspecified => None,
    }
}

fn name_condition_to_condition(filter: &pb::WallpaperStringFilter) -> Option<Condition> {
    let cond =
        pb::StringCondition::try_from(filter.condition).unwrap_or(pb::StringCondition::Unspecified);
    match cond {
        pb::StringCondition::Contains => text_contains_condition(&filter.value),
        pb::StringCondition::ContainsNot => {
            let fts_query = build_fts_match_query(&filter.value)?;
            let quoted = sqlite_quote(&fts_query);
            let esc = filter.value.replace('\'', "''");
            let tag_exists = format!(
                "EXISTS (SELECT 1 FROM item_tag JOIN tag ON tag.id = item_tag.tag_id \
                 WHERE item_tag.item_id = item.id AND tag.name LIKE '%{esc}%' COLLATE NOCASE)"
            );
            let fts = format!(
                "item.id NOT IN (SELECT rowid FROM item_fts WHERE item_fts MATCH {quoted})"
            );
            Some(
                Condition::all()
                    .add(Expr::cust(fts))
                    .add(Expr::cust(format!("NOT {tag_exists}"))),
            )
        }
        _ => string_condition_to_condition(
            || Expr::col((item::Entity, item::Column::DisplayName)),
            cond,
            &filter.value,
            false,
        ),
    }
}

fn text_contains_condition(value: &str) -> Option<Condition> {
    let fts_query = build_fts_match_query(value)?;
    let quoted = sqlite_quote(&fts_query);
    let esc = value.replace('\'', "''");
    let fts = format!("item.id IN (SELECT rowid FROM item_fts WHERE item_fts MATCH {quoted})");
    let tag_exists = format!(
        "EXISTS (SELECT 1 FROM item_tag JOIN tag ON tag.id = item_tag.tag_id \
         WHERE item_tag.item_id = item.id AND tag.name LIKE '%{esc}%' COLLATE NOCASE)"
    );
    Some(
        Condition::any()
            .add(Expr::cust(fts))
            .add(Expr::cust(tag_exists)),
    )
}

fn literal_contains_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('%');
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('%');
    escaped
}

fn build_fts_match_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

fn sqlite_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "''"))
}

fn string_condition_to_condition<E>(
    col: E,
    cond: pb::StringCondition,
    value: &str,
    null_matches_negative: bool,
) -> Option<Condition>
where
    E: Fn() -> sea_orm::sea_query::Expr,
{
    match cond {
        pb::StringCondition::Contains => {
            Some(Condition::all().add(col().like(format!("%{value}%"))))
        }
        pb::StringCondition::ContainsNot => {
            let not_like = col().not_like(format!("%{value}%"));
            if null_matches_negative {
                Some(Condition::any().add(col().is_null()).add(not_like))
            } else {
                Some(Condition::all().add(not_like))
            }
        }
        pb::StringCondition::Is => Some(Condition::all().add(col().eq(value))),
        pb::StringCondition::IsNot => {
            let ne = col().ne(value);
            if null_matches_negative {
                Some(Condition::any().add(col().is_null()).add(ne))
            } else {
                Some(Condition::all().add(ne))
            }
        }
        pb::StringCondition::Unspecified => None,
    }
}

fn int_condition_to_condition<E>(col: E, cond: pb::IntCondition, value: i64) -> Option<Condition>
where
    E: Fn() -> sea_orm::sea_query::Expr,
{
    let expr = match cond {
        pb::IntCondition::Equal => col().eq(value),
        pb::IntCondition::EqualNot => col().ne(value),
        pb::IntCondition::Less => col().lt(value),
        pb::IntCondition::LessEqual => col().lte(value),
        pb::IntCondition::Greater => col().gt(value),
        pb::IntCondition::GreaterEqual => col().gte(value),
        pb::IntCondition::Unspecified => return None,
    };
    Some(Condition::all().add(expr))
}

#[cfg(test)]
mod tests {
    use sea_orm::{EntityTrait, QueryFilter};

    use super::*;
    use crate::model::{
        connect_url,
        entities::item,
        repo::{self, ItemUpsertArgs},
    };

    async fn seed() -> sea_orm::DatabaseConnection {
        let db = connect_url("sqlite::memory:").await.unwrap();
        let plugin = repo::upsert_plugin(&db, "p", "1").await.unwrap();
        let lib_a = repo::add_library(&db, plugin.id, "/lib/a").await.unwrap();
        let lib_b = repo::add_library(&db, plugin.id, "/lib/b").await.unwrap();

        repo::upsert_item(
            &db,
            ItemUpsertArgs {
                plugin_id: plugin.id,
                library_id: lib_a.id,
                path: "city.png",
                ty: "image",
                display_name: "City",
                preview_path: None,
                description: Some("sunset skyline"),
                external_id: Some("wallhaven-Ab_C%42"),
                size: Some(2048),
                width: Some(1920),
                height: Some(1080),
                content_rating: Some("Everyone"),
            },
        )
        .await
        .unwrap();
        repo::upsert_item(
            &db,
            ItemUpsertArgs {
                plugin_id: plugin.id,
                library_id: lib_b.id,
                path: "portrait.webm",
                ty: "video",
                display_name: "Portrait",
                preview_path: None,
                description: None,
                external_id: Some("wallhaven-AbXCDY42"),
                size: Some(4096),
                width: Some(900),
                height: Some(1600),
                content_rating: Some("Mature"),
            },
        )
        .await
        .unwrap();
        repo::upsert_item(
            &db,
            ItemUpsertArgs {
                plugin_id: plugin.id,
                library_id: lib_a.id,
                path: "unrated.png",
                ty: "image",
                display_name: "Unrated",
                preview_path: None,
                description: None,
                external_id: None,
                size: Some(1024),
                width: Some(640),
                height: Some(480),
                content_rating: None,
            },
        )
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn content_rating_exclusion_keeps_unrated_items() {
        let db = seed().await;
        let filter = pb::WallpaperFilterRule {
            r#type: pb::WallpaperFilterType::ContentRating as i32,
            group: 0,
            payload: Some(pb::wallpaper_filter_rule::Payload::StringFilter(
                pb::WallpaperStringFilter {
                    value: "Mature".into(),
                    condition: pb::StringCondition::IsNot as i32,
                },
            )),
        };

        let condition = wallpaper_filters_to_condition(&[filter], &[]).unwrap();
        let rows = item::Entity::find()
            .filter(condition)
            .all(&db)
            .await
            .unwrap();
        let names: BTreeSet<_> = rows.into_iter().map(|row| row.display_name).collect();

        assert_eq!(
            names,
            BTreeSet::from(["City".to_owned(), "Unrated".to_owned()])
        );
    }

    #[tokio::test]
    async fn wallpaper_filters_to_condition_matches_grouped_rules() {
        let db = seed().await;

        let mut name = pb::WallpaperFilterRule {
            r#type: pb::WallpaperFilterType::Name as i32,
            group: 0,
            payload: None,
        };
        name.payload = Some(pb::wallpaper_filter_rule::Payload::StringFilter(
            pb::WallpaperStringFilter {
                value: "City".into(),
                condition: pb::StringCondition::Contains as i32,
            },
        ));

        let mut width = pb::WallpaperFilterRule {
            r#type: pb::WallpaperFilterType::Width as i32,
            group: 0,
            payload: None,
        };
        width.payload = Some(pb::wallpaper_filter_rule::Payload::IntFilter(
            pb::WallpaperIntFilter {
                value: 1000,
                condition: pb::IntCondition::GreaterEqual as i32,
            },
        ));

        let condition = wallpaper_filters_to_condition(&[name, width], &[]).unwrap();
        let rows = item::Entity::find()
            .find_also_related(library::Entity)
            .filter(condition)
            .all(&db)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.display_name, "City");
    }

    #[tokio::test]
    async fn wallpaper_filters_to_condition_honors_group_or_logic() {
        let db = seed().await;

        let mut ty = pb::WallpaperFilterRule {
            r#type: pb::WallpaperFilterType::WpType as i32,
            group: 0,
            payload: None,
        };
        ty.payload = Some(pb::wallpaper_filter_rule::Payload::StringFilter(
            pb::WallpaperStringFilter {
                value: "video".into(),
                condition: pb::StringCondition::Is as i32,
            },
        ));

        let mut wide = pb::WallpaperFilterRule {
            r#type: pb::WallpaperFilterType::Width as i32,
            group: 1,
            payload: None,
        };
        wide.payload = Some(pb::wallpaper_filter_rule::Payload::IntFilter(
            pb::WallpaperIntFilter {
                value: 1500,
                condition: pb::IntCondition::GreaterEqual as i32,
            },
        ));

        let logic = pb::FilterLogic {
            op: pb::LogicOp::Or as i32,
            group_a: 0,
            group_b: 1,
        };

        let condition = wallpaper_filters_to_condition(&[ty, wide], &[logic]).unwrap();
        let rows = item::Entity::find()
            .find_also_related(library::Entity)
            .filter(condition)
            .all(&db)
            .await
            .unwrap();

        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn wallpaper_name_contains_matches_description_via_fts() {
        let db = seed().await;

        let mut name = pb::WallpaperFilterRule {
            r#type: pb::WallpaperFilterType::Name as i32,
            group: 0,
            payload: None,
        };
        name.payload = Some(pb::wallpaper_filter_rule::Payload::StringFilter(
            pb::WallpaperStringFilter {
                value: "skyline".into(),
                condition: pb::StringCondition::Contains as i32,
            },
        ));

        let condition = wallpaper_filters_to_condition(&[name], &[]).unwrap();
        let rows = item::Entity::find()
            .find_also_related(library::Entity)
            .filter(condition)
            .all(&db)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.display_name, "City");
    }

    #[tokio::test]
    async fn wallpaper_search_matches_external_id_literal_substring() {
        let db = seed().await;

        let condition = wallpaper_query_to_condition(&[], &[], "B_C%4").unwrap();
        let rows = item::Entity::find()
            .find_also_related(library::Entity)
            .filter(condition)
            .all(&db)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.display_name, "City");
    }

    #[tokio::test]
    async fn wallpaper_search_ands_with_structured_filters() {
        let db = seed().await;
        let video = pb::WallpaperFilterRule {
            r#type: pb::WallpaperFilterType::WpType as i32,
            group: 0,
            payload: Some(pb::wallpaper_filter_rule::Payload::StringFilter(
                pb::WallpaperStringFilter {
                    value: "video".into(),
                    condition: pb::StringCondition::Is as i32,
                },
            )),
        };

        let condition = wallpaper_query_to_condition(&[video], &[], "B_C%4").unwrap();
        let rows = item::Entity::find()
            .find_also_related(library::Entity)
            .filter(condition)
            .all(&db)
            .await
            .unwrap();

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn wallpaper_search_keeps_existing_text_matches() {
        let db = seed().await;

        let condition = wallpaper_query_to_condition(&[], &[], "skyline").unwrap();
        let rows = item::Entity::find()
            .find_also_related(library::Entity)
            .filter(condition)
            .all(&db)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.display_name, "City");
    }
}
