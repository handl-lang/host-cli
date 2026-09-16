//! Prototype categorical 2-cell rewrite for `hnl transform <rules.hnl> <program.hnl>`.
//!
//! Reads real `~[2]~>` arrow values from the rules file and rewrites the
//! 1-cell `round_trip` term in the program file at the AST/term level.  This
//! is intentionally a minimal working prototype: `flow` (`>>>`) chains are
//! flattened, rules match contiguous subsequences, and the result is emitted
//! by the data-driven formatter.

use handl_kit_canonical::Kit;
use handl_language::{Ast, AstNode, AstRef, LangError};
use handl_plugin_engine_fmt::FormatEngine;
use handl_plugin_engine_lex_pattern;
use handl_plugin_engine_parse::ParserRequest;
use handl_plugin_engine_parse_pratt::PrattEngine;
use serde_json::{Value, json};

/// Source span of a single top-level statement within the original file.
#[derive(Debug, Clone)]
struct StatementInfo {
    source: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy)]
struct Rule {
    ast_index: usize,
    source: AstRef,
    target: AstRef,
}

/// Entry point for the two-file categorical transform.
pub fn run(rules_path: &str, prog_path: &str) -> i32 {
    let kit = match Kit::new() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("transform: {e}");
            return 1;
        }
    };
    let lang = kit.runtime().lang();

    let rules_src = match std::fs::read_to_string(rules_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("transform: failed to read {rules_path}: {e}");
            return 2;
        }
    };
    let prog_src = match std::fs::read_to_string(prog_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("transform: failed to read {prog_path}: {e}");
            return 2;
        }
    };

    let rules_stmts = match parse_raw(lang, &rules_src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("transform: {rules_path}: parse error: {e}");
            return 2;
        }
    };
    let prog_stmts = match parse_raw(lang, &prog_src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("transform: {prog_path}: parse error: {e}");
            return 2;
        }
    };

    let mut rules: Vec<Rule> = Vec::new();
    for (i, (ast, _)) in rules_stmts.iter().enumerate() {
        collect_dim2_rules(i, ast, &mut rules);
    }
    if rules.is_empty() {
        eprintln!("transform: no ~[2]~> rules found in {rules_path}");
        return 1;
    }

    let mut cell = None;
    'outer: for (ast, _) in &prog_stmts {
        for r in 0..ast.len() {
            if let Some(v) = round_trip_value(ast, AstRef::new(r)) {
                cell = Some((ast.clone(), v));
                break 'outer;
            }
        }
    }
    let (prog_ast, cell_root) = match cell {
        Some((ast, r)) => (ast, r),
        None => {
            eprintln!("transform: no `round_trip` 1-cell found in {prog_path}");
            return 1;
        }
    };

    let mut out = Ast::new();
    let new_root = transform_node(&rules, &rules_stmts, &prog_ast, cell_root, &mut out);
    match FormatEngine::new().format(lang, &out, new_root) {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("transform: emit failed: {e}");
            1
        }
    }
}

fn parse_raw(lang: &handl_language::Language, src: &str) -> Result<Vec<(Ast, StatementInfo)>, LangError> {
    let mut asts = Vec::new();
    for info in split_statements(src) {
        let tokens = handl_plugin_engine_lex_pattern::lex(lang, &info.source)?;
        if tokens.as_slice().is_empty() {
            continue;
        }
        let req = ParserRequest::new(tokens);
        let mut ast = PrattEngine::new().parse(lang, &req)?;
        if !ast.is_empty() {
            offset_spans(&mut ast, info.start)?;
            asts.push((ast, info));
        }
    }
    Ok(asts)
}

fn split_statements(src: &str) -> Vec<StatementInfo> {
    let bytes = src.as_bytes();
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut depth: i32 = 0;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];

        // String literal -- skip to the closing quote (respecting escapes).
        if b == b'"' {
            i += 1;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => {
                        i += 2;
                        continue;
                    }
                    b'"' => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            continue;
        }

        // Comment: `--` to end of line.
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }

        if depth == 0 && (b == b'\n' || b == b';') {
            let (trim_start, trim_end) = trim_offsets(src, start, i);
            if trim_start < trim_end {
                chunks.push(StatementInfo {
                    source: src[trim_start..trim_end].to_string(),
                    start: trim_start,
                    end: trim_end,
                });
            }
            i += 1;
            start = i;
            continue;
        }

        i += 1;
    }

    let (trim_start, trim_end) = trim_offsets(src, start, src.len());
    if trim_start < trim_end {
        chunks.push(StatementInfo {
            source: src[trim_start..trim_end].to_string(),
            start: trim_start,
            end: trim_end,
        });
    }
    chunks
}

fn trim_offsets(src: &str, a: usize, b: usize) -> (usize, usize) {
    let bytes = src.as_bytes();
    let mut start = a;
    while start < b && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = b;
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    (start, end)
}

fn offset_spans(ast: &mut Ast, delta: usize) -> Result<(), LangError> {
    for i in 0..ast.len() {
        let r = AstRef::new(i);
        let node = ast
            .get_mut(r)
            .ok_or(LangError::AstIndexOutOfBounds { index: i })?;
        if let Some(span) = node.span.as_mut() {
            span.start += delta;
            span.end += delta;
        }
    }
    Ok(())
}

fn collect_dim2_rules(ast_index: usize, ast: &Ast, rules: &mut Vec<Rule>) {
    for i in 0..ast.len() {
        collect_dim2_rules_at(ast_index, ast, AstRef::new(i), rules);
    }
}

fn collect_dim2_rules_at(ast_index: usize, ast: &Ast, root: AstRef, rules: &mut Vec<Rule>) {
    if let Some(node) = ast.get(root) {
        if node.kind == "dim_arrow" {
            if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
                if children.len() == 3 {
                    if let (Some(s), Some(d), Some(t)) = (
                        children[0].as_u64(),
                        children[1].as_u64(),
                        children[2].as_u64(),
                    ) {
                        if dim_is_two(ast, AstRef::new(d as usize)) {
                            rules.push(Rule {
                                ast_index,
                                source: AstRef::new(s as usize),
                                target: AstRef::new(t as usize),
                            });
                        }
                    }
                }
            }
        }
        if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
            for c in children {
                if let Some(idx) = c.as_u64() {
                    collect_dim2_rules_at(ast_index, ast, AstRef::new(idx as usize), rules);
                }
            }
        }
        if let Some(body) = node.payload.get("body").and_then(|b| b.as_u64()) {
            collect_dim2_rules_at(ast_index, ast, AstRef::new(body as usize), rules);
        }
    }
}

fn dim_is_two(ast: &Ast, r: AstRef) -> bool {
    ast.get(r).map_or(false, |n| {
        if n.kind == "value" {
            if let Some(v) = n.payload.get("value") {
                if let Some(n) = v.as_u64() {
                    return n == 2;
                }
            }
        }
        false
    })
}

fn round_trip_value(ast: &Ast, root: AstRef) -> Option<AstRef> {
    let node = ast.get(root)?;
    if node.kind == "assign" {
        if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
            if children.len() >= 2 {
                let target = children[0].as_u64()? as usize;
                let value = children[1].as_u64()? as usize;
                if let Some(tnode) = ast.get(AstRef::new(target)) {
                    if tnode.kind == "var" {
                        if let Some(name) = tnode.payload.get("name").and_then(|n| n.as_str()) {
                            if name == "round_trip" {
                                return Some(AstRef::new(value));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn transform_node(
    rules: &[Rule],
    rules_stmts: &[(Ast, StatementInfo)],
    in_ast: &Ast,
    in_root: AstRef,
    out: &mut Ast,
) -> AstRef {
    let in_node = match in_ast.get(in_root) {
        Some(n) => n,
        None => return in_root,
    };

    if in_node.kind == "flow" {
        let in_chain = flatten_flow(in_ast, in_root);
        let mut trans: Vec<AstRef> = Vec::with_capacity(in_chain.len());
        for r in in_chain {
            trans.push(transform_node(rules, rules_stmts, in_ast, r, out));
        }
        let final_chain = apply_rules_to_chain(rules, rules_stmts, out, &trans);
        return build_flow(out, &final_chain);
    }

    let mut new_payload = in_node.payload.clone();
    if let Some(children) = new_payload.get_mut("children").and_then(|c| c.as_array_mut()) {
        for i in 0..children.len() {
            if let Some(idx) = children[i].as_u64() {
                let child_ref = AstRef::new(idx as usize);
                let new_child = transform_node(rules, rules_stmts, in_ast, child_ref, out);
                children[i] = Value::from(new_child.index() as u64);
            }
        }
    }
    if let Some(body) = new_payload.get_mut("body").and_then(|b| b.as_u64()) {
        let body_ref = AstRef::new(body as usize);
        let new_body = transform_node(rules, rules_stmts, in_ast, body_ref, out);
        if let Some(obj) = new_payload.as_object_mut() {
            obj.insert("body".to_string(), Value::from(new_body.index() as u64));
        }
    }

    let new_ref = out.push(AstNode {
        kind: in_node.kind.clone(),
        span: None,
        payload: new_payload,
    });

    for rule in rules {
        let rule_ast = &rules_stmts[rule.ast_index].0;
        if matches_rule(rule_ast, rule.source, out, new_ref) {
            return copy_subtree(rule_ast, rule.target, out);
        }
    }
    new_ref
}

fn flatten_flow(ast: &Ast, root: AstRef) -> Vec<AstRef> {
    if let Some(node) = ast.get(root) {
        if node.kind == "flow" {
            if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
                if children.len() == 2 {
                    if let (Some(l), Some(r)) = (children[0].as_u64(), children[1].as_u64()) {
                        let mut res = flatten_flow(ast, AstRef::new(l as usize));
                        res.extend(flatten_flow(ast, AstRef::new(r as usize)));
                        return res;
                    }
                }
            }
        }
    }
    vec![root]
}

fn build_flow(out: &mut Ast, chain: &[AstRef]) -> AstRef {
    if chain.len() == 1 {
        return chain[0];
    }
    let right = build_flow(out, &chain[1..]);
    let payload = json!({"children": [chain[0].index() as u64, right.index() as u64]});
    out.push(AstNode {
        kind: "flow".to_string(),
        span: None,
        payload,
    })
}

fn apply_rules_to_chain(
    rules: &[Rule],
    rules_stmts: &[(Ast, StatementInfo)],
    out: &mut Ast,
    chain: &[AstRef],
) -> Vec<AstRef> {
    let mut cur = chain.to_vec();
    let mut changed = true;
    while changed {
        changed = false;
        for rule in rules {
            let rule_ast = &rules_stmts[rule.ast_index].0;
            let src_flat = flatten_flow(rule_ast, rule.source);
            if src_flat.is_empty() {
                continue;
            }
            for start in 0..=cur.len().saturating_sub(src_flat.len()) {
                let slice = &cur[start..start + src_flat.len()];
                let mut ok = true;
                for (i, s) in src_flat.iter().enumerate() {
                    if !match_ast(rule_ast, *s, out, slice[i]) {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    let tgt_ref = copy_subtree(rule_ast, rule.target, out);
                    let mut tgt_chain = if out.get(tgt_ref).map(|n| n.kind == "flow").unwrap_or(false) {
                        flatten_flow(out, tgt_ref)
                    } else {
                        vec![tgt_ref]
                    };
                    let mut new_chain = cur[..start].to_vec();
                    new_chain.extend(tgt_chain);
                    new_chain.extend_from_slice(&cur[start + src_flat.len()..]);
                    cur = new_chain;
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }
        }
    }
    cur
}

fn matches_rule(rule_ast: &Ast, src: AstRef, out: &Ast, out_ref: AstRef) -> bool {
    let src_flat = flatten_flow(rule_ast, src);
    let out_flat = flatten_flow(out, out_ref);
    if src_flat.len() != out_flat.len() {
        return false;
    }
    for (i, s) in src_flat.iter().enumerate() {
        if !match_ast(rule_ast, *s, out, out_flat[i]) {
            return false;
        }
    }
    true
}

fn match_ast(a_ast: &Ast, a: AstRef, b_ast: &Ast, b: AstRef) -> bool {
    let an = match a_ast.get(a) {
        Some(n) => n,
        None => return false,
    };
    let bn = match b_ast.get(b) {
        Some(n) => n,
        None => return false,
    };
    if an.kind != bn.kind {
        return false;
    }
    compare_payload(a_ast, &an.payload, b_ast, &bn.payload)
}

fn compare_payload(a_ast: &Ast, a_payload: &Value, b_ast: &Ast, b_payload: &Value) -> bool {
    match (a_payload, b_payload) {
        (Value::Array(a_arr), Value::Array(b_arr)) => {
            if a_arr.len() != b_arr.len() {
                return false;
            }
            for (i, av) in a_arr.iter().enumerate() {
                if let (Some(ai), Some(bi)) = (av.as_u64(), b_arr[i].as_u64()) {
                    if !match_ast(a_ast, AstRef::new(ai as usize), b_ast, AstRef::new(bi as usize)) {
                        return false;
                    }
                } else if av != &b_arr[i] {
                    return false;
                }
            }
            true
        }
        (Value::Object(a_obj), Value::Object(b_obj)) => {
            if a_obj.len() != b_obj.len() {
                return false;
            }
            for (k, v) in a_obj {
                let bv = match b_obj.get(k) {
                    Some(x) => x,
                    None => return false,
                };
                if k == "children" {
                    if let (Some(a_arr), Some(b_arr)) = (v.as_array(), bv.as_array()) {
                        if a_arr.len() != b_arr.len() {
                            return false;
                        }
                        for (i, av) in a_arr.iter().enumerate() {
                            if let (Some(ai), Some(bi)) = (av.as_u64(), b_arr[i].as_u64()) {
                                if !match_ast(a_ast, AstRef::new(ai as usize), b_ast, AstRef::new(bi as usize)) {
                                    return false;
                                }
                            } else if av != &b_arr[i] {
                                return false;
                            }
                        }
                    } else {
                        return false;
                    }
                } else if k == "body" {
                    if let (Some(ai), Some(bi)) = (v.as_u64(), bv.as_u64()) {
                        if !match_ast(a_ast, AstRef::new(ai as usize), b_ast, AstRef::new(bi as usize)) {
                            return false;
                        }
                    } else if v != bv {
                        return false;
                    }
                } else if v != bv {
                    return false;
                }
            }
            true
        }
        _ => a_payload == b_payload,
    }
}

fn copy_subtree(src_ast: &Ast, src_ref: AstRef, out: &mut Ast) -> AstRef {
    let sub = src_ast.subtree(src_ref).expect("valid rule target");
    let base = out.len();
    for n in sub.nodes() {
        out.push(n.clone());
    }
    AstRef::new(base + sub.len() - 1)
}
