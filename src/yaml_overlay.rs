use std::borrow::Cow;

use anyhow::{Result, anyhow};
use yamlpatch::{Op, Patch, apply_yaml_patches};
use yamlpath::{Document, FeatureKind, QueryError, Route};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverlayMode {
    Overwrite,
    Prepend,
    Append,
}

pub fn render_yaml(source_content: &str, override_content: &str) -> Result<String> {
    let override_trimmed = override_content.trim();
    if override_trimmed.is_empty() || override_trimmed == "null" {
        return Ok(source_content.to_string());
    }

    let source_doc = Document::new(source_content)?;
    let overlay_doc = Document::new(override_content)?;
    let overlay_value: serde_yaml::Value = serde_yaml::from_str(override_content)?;

    match overlay_value {
        serde_yaml::Value::Mapping(root) => {
            let result = apply_mapping(
                source_doc,
                &Route::default(),
                &root,
                &overlay_doc,
                &Route::default(),
            )?;
            let mut rendered = result.source().to_string();
            if !rendered.ends_with('\n') {
                rendered.push('\n');
            }
            Ok(rendered)
        }
        serde_yaml::Value::Null => Ok(source_content.to_string()),
        _ => Err(anyhow!("override 根节点必须是对象")),
    }
}

fn apply_mapping(
    mut document: Document,
    parent_route: &Route<'_>,
    mapping: &serde_yaml::Mapping,
    overlay_doc: &Document,
    overlay_parent_route: &Route<'_>,
) -> Result<Document> {
    for (raw_key_value, value) in mapping {
        let raw_key = key_to_string(raw_key_value)?;
        let (clean_key, mode) = parse_overlay_mode(&raw_key);

        let target_route = parent_route.with_key(clean_key.clone());
        let overlay_raw_route = overlay_parent_route.with_key(raw_key);

        document = apply_entry(
            document,
            parent_route,
            &target_route,
            &clean_key,
            value,
            mode,
            overlay_doc,
            &overlay_raw_route,
        )?;
    }

    Ok(document)
}

#[allow(clippy::too_many_arguments)]
fn apply_entry(
    mut document: Document,
    parent_route: &Route<'_>,
    target_route: &Route<'_>,
    clean_key: &str,
    value: &serde_yaml::Value,
    mode: OverlayMode,
    overlay_doc: &Document,
    overlay_raw_route: &Route<'_>,
) -> Result<Document> {
    let existing_feature = query_feature(&document, target_route)?;

    match mode {
        OverlayMode::Overwrite => {
            let recurse = matches!(value, serde_yaml::Value::Mapping(_))
                && matches!(
                    existing_feature.as_ref().map(|f| f.kind()),
                    Some(FeatureKind::BlockMapping | FeatureKind::FlowMapping)
                );

            if recurse {
                if let serde_yaml::Value::Mapping(child_mapping) = value {
                    document = apply_mapping(
                        document,
                        target_route,
                        child_mapping,
                        overlay_doc,
                        overlay_raw_route,
                    )?;
                }
            } else {
                document = upsert_value(document, parent_route, target_route, clean_key, value)?;
            }

            apply_overlay_comment(document, target_route, overlay_doc, overlay_raw_route)
        }
        OverlayMode::Prepend | OverlayMode::Append => match existing_feature {
            Some(feature) => match feature.kind() {
                FeatureKind::BlockSequence => {
                    if let serde_yaml::Value::Sequence(items) = value {
                        document = update_block_sequence(document, target_route, items, mode)?;

                        apply_overlay_comment(
                            document,
                            target_route,
                            overlay_doc,
                            overlay_raw_route,
                        )
                    } else {
                        document = replace_value(document, target_route, value)?;
                        apply_overlay_comment(
                            document,
                            target_route,
                            overlay_doc,
                            overlay_raw_route,
                        )
                    }
                }
                FeatureKind::FlowSequence => {
                    if let serde_yaml::Value::Sequence(items) = value {
                        document =
                            prepend_or_append_flow_sequence(document, target_route, items, mode)?;
                        apply_overlay_comment(
                            document,
                            target_route,
                            overlay_doc,
                            overlay_raw_route,
                        )
                    } else {
                        document = replace_value(document, target_route, value)?;
                        apply_overlay_comment(
                            document,
                            target_route,
                            overlay_doc,
                            overlay_raw_route,
                        )
                    }
                }
                FeatureKind::BlockMapping | FeatureKind::FlowMapping => {
                    if let serde_yaml::Value::Mapping(child_mapping) = value {
                        document = apply_mapping(
                            document,
                            target_route,
                            child_mapping,
                            overlay_doc,
                            overlay_raw_route,
                        )?;
                        apply_overlay_comment(
                            document,
                            target_route,
                            overlay_doc,
                            overlay_raw_route,
                        )
                    } else {
                        document = replace_value(document, target_route, value)?;
                        apply_overlay_comment(
                            document,
                            target_route,
                            overlay_doc,
                            overlay_raw_route,
                        )
                    }
                }
                FeatureKind::Scalar => {
                    document = replace_value(document, target_route, value)?;
                    apply_overlay_comment(document, target_route, overlay_doc, overlay_raw_route)
                }
            },
            None => {
                document = add_value(document, parent_route, clean_key, value)?;
                apply_overlay_comment(document, target_route, overlay_doc, overlay_raw_route)
            }
        },
    }
}

fn update_block_sequence(
    document: Document,
    route: &Route<'_>,
    items: &[serde_yaml::Value],
    mode: OverlayMode,
) -> Result<Document> {
    let existing = read_existing_sequence(&document, route)?;

    let merged = if mode == OverlayMode::Prepend {
        let mut out = items.to_vec();
        out.extend(existing);
        out
    } else {
        let mut out = existing;
        out.extend(items.iter().cloned());
        out
    };

    replace_block_sequence_at_route(&document, route, &merged)
}

fn prepend_or_append_flow_sequence(
    document: Document,
    route: &Route<'_>,
    items: &[serde_yaml::Value],
    mode: OverlayMode,
) -> Result<Document> {
    let Some(feature) = query_feature(&document, route)? else {
        return Ok(document);
    };

    let current_content = document.extract_with_leading_whitespace(&feature);
    let current_value: serde_yaml::Value = serde_yaml::from_str(current_content)?;

    let serde_yaml::Value::Sequence(existing) = current_value else {
        return replace_value(
            document,
            route,
            &serde_yaml::Value::Sequence(items.to_vec()),
        );
    };

    let merged = if mode == OverlayMode::Prepend {
        let mut combined = items.to_vec();
        combined.extend(existing);
        combined
    } else {
        let mut combined = existing;
        combined.extend(items.iter().cloned());
        combined
    };

    replace_value(document, route, &serde_yaml::Value::Sequence(merged))
}

fn upsert_value(
    document: Document,
    parent_route: &Route<'_>,
    target_route: &Route<'_>,
    key: &str,
    value: &serde_yaml::Value,
) -> Result<Document> {
    if query_feature(&document, target_route)?.is_some() {
        if let serde_yaml::Value::Sequence(items) = value
            && query_feature_pretty(&document, target_route)?.is_some()
        {
            return replace_block_sequence_at_route(&document, target_route, items);
        }

        replace_value(document, target_route, value)
    } else {
        add_value(document, parent_route, key, value)
    }
}

fn replace_value(
    document: Document,
    route: &Route<'_>,
    value: &serde_yaml::Value,
) -> Result<Document> {
    apply_single_patch(
        &document,
        Patch {
            route: route.clone(),
            operation: Op::Replace(value.clone()),
        },
    )
}

fn add_value(
    document: Document,
    parent_route: &Route<'_>,
    key: &str,
    value: &serde_yaml::Value,
) -> Result<Document> {
    apply_single_patch(
        &document,
        Patch {
            route: parent_route.clone(),
            operation: Op::Add {
                key: key.to_string(),
                value: value.clone(),
            },
        },
    )
}

fn apply_overlay_comment(
    document: Document,
    target_route: &Route<'_>,
    overlay_doc: &Document,
    overlay_raw_route: &Route<'_>,
) -> Result<Document> {
    let Some(comment) = comment_from_overlay(overlay_doc, overlay_raw_route)? else {
        return Ok(document);
    };

    apply_single_patch(
        &document,
        Patch {
            route: target_route.clone(),
            operation: Op::EmplaceComment {
                new: Cow::Owned(comment),
            },
        },
    )
}

fn comment_from_overlay(overlay_doc: &Document, route: &Route<'_>) -> Result<Option<String>> {
    let Some(feature) = query_feature(overlay_doc, route)? else {
        return Ok(None);
    };

    let comments = overlay_doc.feature_comments(&feature);
    match comments.len() {
        0 => Ok(None),
        1 => Ok(Some(overlay_doc.extract(&comments[0]).to_string())),
        _ => Err(anyhow!(
            "覆写项 {:?} 存在多个注释，无法确定要保留哪一个",
            route
        )),
    }
}

fn replace_block_sequence_at_route(
    document: &Document,
    route: &Route<'_>,
    items: &[serde_yaml::Value],
) -> Result<Document> {
    let feature = query_feature_pretty(document, route)?
        .ok_or_else(|| anyhow!("no existing sequence at {:?}", route))?;

    let serialized = serde_yaml::to_string(&serde_yaml::Value::Sequence(items.to_vec()))?;
    let body = serialized.trim_end();

    let existing = document.extract_with_leading_whitespace(&feature);
    let mut existing_lines = existing.lines();
    let key_line = existing_lines
        .next()
        .ok_or_else(|| anyhow!("无效的序列节点内容"))?
        .to_string();

    let key_indent = leading_spaces(&key_line);
    let item_indent = existing_lines
        .find(|line| line.trim_start().starts_with('-'))
        .map(leading_spaces)
        .unwrap_or_else(|| format!("{key_indent}    "));

    let list_lines = body
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{item_indent}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let rewritten = format!("{key_line}\n{list_lines}");

    let mut source = document.source().to_string();
    source.replace_range(
        feature.location.byte_span.0..feature.location.byte_span.1,
        &rewritten,
    );
    Document::new(&source)
        .map_err(|err| anyhow!("序列替换后 YAML 非法: {err}\n----\n{source}\n----"))
}

fn read_existing_sequence(
    document: &Document,
    route: &Route<'_>,
) -> Result<Vec<serde_yaml::Value>> {
    let feature = query_feature_pretty(document, route)?
        .ok_or_else(|| anyhow!("no existing sequence at {:?}", route))?;

    let content = document.extract_with_leading_whitespace(&feature);
    let parsed: serde_yaml::Value = serde_yaml::from_str(content)?;

    match parsed {
        serde_yaml::Value::Mapping(mapping) => {
            let Some((_, value)) = mapping.into_iter().next() else {
                return Ok(Vec::new());
            };
            if let serde_yaml::Value::Sequence(items) = value {
                Ok(items)
            } else {
                Err(anyhow!("目标不是序列"))
            }
        }
        serde_yaml::Value::Sequence(items) => Ok(items),
        _ => Err(anyhow!("目标不是序列")),
    }
}

fn leading_spaces(line: &str) -> String {
    line.chars()
        .take_while(|ch| ch.is_ascii_whitespace())
        .collect::<String>()
}

fn query_feature<'a>(
    doc: &'a Document,
    route: &Route<'_>,
) -> Result<Option<yamlpath::Feature<'a>>> {
    match doc.query_exact(route) {
        Ok(feature) => Ok(feature),
        Err(QueryError::ExhaustedMapping(_)) | Err(QueryError::ExhaustedList(_, _)) => Ok(None),
        Err(err) => Err(anyhow!(err.to_string())),
    }
}

fn query_feature_pretty<'a>(
    doc: &'a Document,
    route: &Route<'_>,
) -> Result<Option<yamlpath::Feature<'a>>> {
    match doc.query_pretty(route) {
        Ok(feature) => Ok(Some(feature)),
        Err(QueryError::ExhaustedMapping(_)) | Err(QueryError::ExhaustedList(_, _)) => Ok(None),
        Err(err) => Err(anyhow!(err.to_string())),
    }
}

fn apply_single_patch(document: &Document, patch: Patch<'_>) -> Result<Document> {
    apply_yaml_patches(document, &[patch]).map_err(|e| anyhow!(e.to_string()))
}

fn parse_overlay_mode(raw_key: &str) -> (String, OverlayMode) {
    if let Some(rest) = raw_key.strip_prefix('+') {
        return (rest.to_string(), OverlayMode::Prepend);
    }

    if let Some(rest) = raw_key.strip_suffix('+') {
        return (rest.to_string(), OverlayMode::Append);
    }

    (raw_key.to_string(), OverlayMode::Overwrite)
}

fn key_to_string(key: &serde_yaml::Value) -> Result<String> {
    match key {
        serde_yaml::Value::String(s) => Ok(s.clone()),
        serde_yaml::Value::Bool(b) => Ok(b.to_string()),
        serde_yaml::Value::Number(n) => Ok(n.to_string()),
        serde_yaml::Value::Null => Ok("null".to_string()),
        _ => Err(anyhow!("不支持复杂类型作为覆写键")),
    }
}

#[cfg(test)]
mod tests {
    use super::render_yaml;

    const SOURCE_BASE: &str = r#"
# 全局基础配置注释
tun: # tun 行内注释
    enable: false # enable 注释
    mtu: 1500
    stack: gvisor
dns:
    enable: true
    nameserver:
        - 1.1.1.1 # nameserver 注释
rules:
    - MATCH,DIRECT
"#;

    mod functional_cases {
        use super::{SOURCE_BASE, render_yaml};

        #[test]
        fn prepend_sequence_with_plus_prefix() {
            let overlay = r#"
+rules: # 新注释
    - GEOIP,CN,DIRECT
    - GEOIP,GOOGLE,PROXY
"#;
            let expected = r#"
# 全局基础配置注释
tun: # tun 行内注释
    enable: false # enable 注释
    mtu: 1500
    stack: gvisor
dns:
    enable: true
    nameserver:
        - 1.1.1.1 # nameserver 注释
rules:
    - GEOIP,CN,DIRECT
    - GEOIP,GOOGLE,PROXY
    - MATCH,DIRECT
"#;
            let rendered = render_yaml(SOURCE_BASE, overlay).expect("render failed");
            assert_eq!(rendered, expected);
        }

        #[test]
        fn append_sequence_with_plus_suffix() {
            let overlay = r#"
rules+: # 新注释
    - GEOIP,CN,DIRECT
    - GEOIP,GOOGLE,PROXY
"#;
            let expected = r#"
# 全局基础配置注释
tun: # tun 行内注释
    enable: false # enable 注释
    mtu: 1500
    stack: gvisor
dns:
    enable: true
    nameserver:
        - 1.1.1.1 # nameserver 注释
rules:
    - MATCH,DIRECT
    - GEOIP,CN,DIRECT
    - GEOIP,GOOGLE,PROXY
"#;
            let rendered = render_yaml(SOURCE_BASE, overlay).expect("render failed");
            assert_eq!(rendered, expected);
        }

        #[test]
        fn plus_mode_merges_mapping_and_respects_array_direction() {
            let overlay = r#"
+dns: # 新注释
    enhanced-mode: fake-ip
"#;
            let expected = r#"
# 全局基础配置注释
tun: # tun 行内注释
    enable: false # enable 注释
    mtu: 1500
    stack: gvisor
dns:
    enable: true
    nameserver:
        - 1.1.1.1 # nameserver 注释
    enhanced-mode: fake-ip
rules:
    - MATCH,DIRECT
"#;
            let rendered = render_yaml(SOURCE_BASE, overlay).expect("render failed");
            assert_eq!(rendered, expected);
        }

        #[test]
        fn overwrite_mode_updates_target_and_keeps_unrelated_content() {
            let overlay = r#"
tun: # 新注释
    enable: true
"#;

            let expected = r#"
# 全局基础配置注释
tun: # tun 行内注释
    enable: true # enable 注释
    mtu: 1500
    stack: gvisor
dns:
    enable: true
    nameserver:
        - 1.1.1.1 # nameserver 注释
rules:
    - MATCH,DIRECT
"#;

            let rendered = render_yaml(SOURCE_BASE, overlay).expect("render failed");
            assert_eq!(&rendered, expected);
        }

        #[test]
        fn overwrite_array_replaces_instead_of_merge() {
            let overlay = r#"
rules:
    - GEOIP,CN,DIRECT
    - MATCH,PROXY
"#;

            let expected = r#"
# 全局基础配置注释
tun: # tun 行内注释
    enable: false # enable 注释
    mtu: 1500
    stack: gvisor
dns:
    enable: true
    nameserver:
        - 1.1.1.1 # nameserver 注释
rules:
    - GEOIP,CN,DIRECT
    - MATCH,PROXY
"#;

            let rendered = render_yaml(SOURCE_BASE, overlay).expect("render failed");
            assert_eq!(&rendered, expected);
        }

        #[test]
        fn supports_non_string_keys() {
            let source = r#"
true: old # 行内注释
100: old
"#;

            let overlay = r#"
true: new
100: 200
"#;

            let expected = r#"
true: new # 行内注释
100: 200
"#;

            let rendered = render_yaml(source, overlay).expect("render failed");
            assert_eq!(&rendered, expected);
        }
    }

    mod edge_cases {
        use super::{SOURCE_BASE, render_yaml};

        #[test]
        fn keeps_source_when_override_is_empty() {
            let rendered = render_yaml(SOURCE_BASE, "").expect("render failed");
            assert_eq!(rendered, SOURCE_BASE);
        }

        #[test]
        fn keeps_source_when_override_is_null() {
            let overlay = r#"
null
"#;

            let rendered = render_yaml(SOURCE_BASE, overlay).expect("render failed");
            assert_eq!(rendered, SOURCE_BASE);
        }

        #[test]
        fn plus_mode_on_scalar_falls_back_to_replace() {
            let source = r#"
mode: direct # 老注释
"#;

            let overlay = r#"
+mode: rule # 新注释
"#;

            let expected = r#"
mode: rule # 新注释
"#;

            let rendered = render_yaml(source, overlay).expect("render failed");
            assert_eq!(rendered, expected);
        }

        #[test]
        fn plus_mode_adds_when_key_missing() {
            let source = r#"
dns:
    enable: true
"#;

            let overlay = r#"
dns:
    +fallback:
        - 8.8.8.8
"#;

            let expected = r#"
dns:
    enable: true
    fallback: ["8.8.8.8"]
"#;

            let rendered = render_yaml(source, overlay).expect("render failed");
            assert_eq!(rendered, expected);
        }
    }
}
