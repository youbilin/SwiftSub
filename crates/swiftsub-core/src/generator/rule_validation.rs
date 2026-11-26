use crate::{
    Rule,
    error::{Error, Result},
};
use serde_yaml_ng::Value;
use std::collections::{HashMap, HashSet};

/// Validate the references used by both generated configurations and the saved YAML editor.
pub fn validate_rule_definitions(
    rules: &[Rule],
    definitions: Option<&Value>,
    targets: &HashSet<String>,
    providers: &HashSet<String>,
) -> Result<()> {
    let mut sub_rules = HashMap::<String, Vec<Rule>>::new();
    if let Some(definitions) = definitions {
        let mapping = definitions.as_mapping().ok_or_else(|| {
            Error::InvalidConfig("sub-rules 必须是子规则名称与规则列表组成的对象".into())
        })?;
        for (name, entries) in mapping {
            let name = name
                .as_str()
                .filter(|name| !name.trim().is_empty() && !name.contains([',', '\n', '\r']))
                .ok_or_else(|| Error::InvalidConfig("子规则需要有效的文本名称".into()))?;
            let entries = entries
                .as_sequence()
                .ok_or_else(|| Error::InvalidConfig(format!("子规则「{name}」必须是规则列表")))?;
            let mut parsed = Vec::new();
            for (index, entry) in entries.iter().enumerate() {
                let line = entry.as_str().ok_or_else(|| {
                    Error::InvalidConfig(format!(
                        "子规则「{name}」第 {} 条必须是规则文本",
                        index + 1
                    ))
                })?;
                if Rule::parts(line)
                    .first()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("FINAL"))
                {
                    return Err(Error::InvalidConfig(format!(
                        "子规则「{name}」第 {} 条「{line}」：Mihomo 的兜底规则应使用 MATCH",
                        index + 1
                    )));
                }
                parsed.push(Rule::parse(line).map_err(|error| {
                    Error::InvalidConfig(format!(
                        "子规则「{name}」第 {} 条「{line}」：{error}",
                        index + 1
                    ))
                })?);
            }
            sub_rules.insert(name.into(), parsed);
        }
    }
    let validate = |rule: &Rule, owner: &str| -> Result<()> {
        if rule.kind == "SUB-RULE" {
            if !sub_rules.contains_key(&rule.target) {
                return Err(Error::InvalidConfig(format!(
                    "{owner} 引用未定义的子规则：{}",
                    rule.target
                )));
            }
        } else if !targets.contains(&rule.target)
            && ![
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "PASS",
                "COMPATIBLE",
                "GLOBAL",
            ]
            .contains(&rule.target.as_str())
        {
            return Err(Error::InvalidConfig(format!(
                "{owner} 引用不存在的目标：{}",
                rule.target
            )));
        }
        for provider in rule.referenced_rule_sets() {
            if !providers.contains(&provider) {
                return Err(Error::InvalidConfig(format!(
                    "{owner} 引用未定义的规则集：{provider}"
                )));
            }
        }
        Ok(())
    };
    for (index, rule) in rules.iter().enumerate() {
        validate(
            rule,
            &format!("规则第 {} 条「{}」", index + 1, rule.to_line()),
        )?;
    }
    let mut names: Vec<_> = sub_rules.keys().collect();
    names.sort();
    for name in &names {
        for (index, rule) in sub_rules[*name].iter().enumerate() {
            validate(
                rule,
                &format!("子规则「{name}」第 {} 条「{}」", index + 1, rule.to_line()),
            )?;
        }
    }
    fn visit<'a>(
        name: &'a str,
        sub_rules: &'a HashMap<String, Vec<Rule>>,
        active: &mut HashSet<&'a str>,
        done: &mut HashSet<&'a str>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        if !active.insert(name) {
            return Err(Error::InvalidConfig(format!("子规则循环引用：{name}")));
        }
        if active.len() > 64 {
            return Err(Error::InvalidConfig(format!("子规则引用层级过深：{name}")));
        }
        for rule in &sub_rules[name] {
            if rule.kind == "SUB-RULE" {
                visit(&rule.target, sub_rules, active, done)?;
            }
        }
        active.remove(name);
        done.insert(name);
        Ok(())
    }
    let mut active = HashSet::new();
    let mut done = HashSet::new();
    for name in names {
        visit(name, &sub_rules, &mut active, &mut done)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_sub_rules_validate_targets_providers_and_cycles_with_owners() {
        let rules = vec![Rule::parse("SUB-RULE,(DOMAIN-SUFFIX,example.org),first").unwrap()];
        let targets = HashSet::from(["node".into()]);
        let providers = HashSet::from(["domains".into()]);
        let mut definitions: Value = serde_yaml_ng::from_str("first: ['SUB-RULE,(NETWORK,TCP),second']\nsecond: ['OR,((RULE-SET,domains),(NETWORK,UDP)),node', 'MATCH,DIRECT']").unwrap();
        validate_rule_definitions(&rules, Some(&definitions), &targets, &providers).unwrap();
        let error =
            validate_rule_definitions(&rules, Some(&definitions), &targets, &HashSet::new())
                .unwrap_err()
                .to_string();
        assert!(
            error.contains("second") && error.contains("domains"),
            "{error}"
        );
        definitions["second"][0] = Value::from("DOMAIN,example.org,missing-node");
        let error = validate_rule_definitions(&rules, Some(&definitions), &targets, &providers)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("second") && error.contains("missing-node"),
            "{error}"
        );
        definitions["second"][0] = Value::from("SUB-RULE,(NETWORK,TCP),first");
        assert!(
            validate_rule_definitions(&rules, Some(&definitions), &targets, &providers)
                .unwrap_err()
                .to_string()
                .contains("循环")
        );
        definitions["second"][0] = Value::from("aaa,example.org,DIRECT");
        let error = validate_rule_definitions(&rules, Some(&definitions), &targets, &providers)
            .unwrap_err()
            .to_string();
        assert!(error.contains("second") && error.contains("aaa"), "{error}");
    }
}
