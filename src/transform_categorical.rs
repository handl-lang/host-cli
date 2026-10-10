//! `~[2]~>` categorical arrow optimization behind the `NativeRewrite` seam.
//!
//! Provides `n_dim_arrow_opt`, a native rewrite for the `n_dim_opt` transform
//! pass.  The same native backs `hnl run` (optimize the module's own 1-cell
//! `flow`s before eval) and the two-file `hnl transform <rules.hnl>
//! <program.hnl>` command.
//!
//! Child positions are resolved through `Language::child_role_index` (ADR-0081)
//! where possible; the few remaining shape assumptions (`children`/`body`
//! payload keys, `flow` left/right, `value` literal) are language-data facts,
//! not hardcoded node-kind branches in the engine core.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use handl_kit_canonical::Kit;
use handl_language::ast::ARENA_REF_PAYLOAD_KEYS;
use handl_language::{Ast, AstNode, AstRef, LangError, Language};
use handl_plugin_engine_fmt::FormatEngine;
use handl_plugin_engine_lex_pattern;
use handl_plugin_engine_parse::{ParserEngine, ParserRequest};
use handl_plugin_engine_parse_pratt::PrattEngine;
use handl_plugin_engine_transform::native::{NativeRewrite, NativeRewriteRegistry, RewriteCtx};
use handl_plugin_engine_transform_rewrite::RewriteEngine;
use serde_json::{Value, json};

/// The dimension a `~[N]~>` rule fires at.
///
/// - `Fixed(k)` rewrites chains of `(k-1)`-cells only.
/// - `Any` (a symbolic `~[N]~>` dimension variable) fires at every
///   dimension — the same law governs programs, rules, proofs, …
#[derive(Clone, Debug)]
pub enum RuleDim {
    Fixed(u64),
    Any,
}

/// A single `~[N]~>` rule extracted from a `dim_arrow` surface node.
///
/// `metavars` are the free `var` names in the source/target (names not
/// bound anywhere in the rules module): they are pattern variables that
/// match any cell and must bind consistently.
#[derive(Clone)]
pub struct Rule {
    source_ast: Arc<Ast>,
    source: AstRef,
    target: AstRef,
    dim: RuleDim,
    metavars: HashSet<String>,
}

/// Build a `NativeRewrite` that implements `n_dim_arrow_opt` using `rules`.
///
/// `protected` is the set of input-arena indices that are *quoted* — terms
/// inside `~[N]~>` sources and targets are syntax being talked about, not
/// code to be rewritten — so the pass leaves them alone.
pub fn native(
    rules: Vec<Rule>,
    protected: Arc<HashSet<usize>>,
    consts: Arc<HashMap<String, u64>>,
) -> Arc<dyn NativeRewrite> {
    let rules = Arc::new(rules);
    let mut registry = NativeRewriteRegistry::new();
    registry.register(
        "n_dim_arrow_opt",
        Box::new(move |ctx: &mut RewriteCtx<'_>| n_dim_arrow_opt(ctx, &rules, &protected, &consts)),
    );
    Arc::new(registry)
}

/// Whether `kind` is the arrow form, via the `arrow` face (ADR-0123),
/// falling back to the canonical `dim_arrow` for face-less languages/kits.
///
/// The dimension-analysis pass is written against the arrow shape; naming the
/// face instead of the kind means a language that spells it differently still
/// works, and keeps the four checks in this file consistent.
fn is_arrow_kind(lang: &Language, kind: &str) -> bool {
    if lang.declares_faces(kind) {
        lang.shape_has_face(kind, "arrow")
    } else {
        kind == "dim_arrow"
    }
}

/// Indices of every node inside a `dim_arrow` node (the `~[N]~>` source,
/// dimension, and target subtrees). The optimizer pass must not rewrite
/// these: they are the quoted terms rules and certificates talk about.
pub fn dim_arrow_subtrees(ast: &Ast, lang: &Language) -> HashSet<usize> {
    let mut protected = HashSet::new();
    for i in 0..ast.len() {
        let r = AstRef::new(i);
        if ast.get(r).is_some_and(|n| is_arrow_kind(lang, &n.kind)) {
            for c in child_refs(ast.get(r).unwrap()) {
                collect_descendants(ast, c, &mut protected);
            }
        }
    }
    protected
}

fn collect_descendants(ast: &Ast, r: AstRef, out: &mut HashSet<usize>) {
    if !out.insert(r.index()) {
        return;
    }
    if let Some(node) = ast.get(r) {
        for c in child_refs(node) {
            collect_descendants(ast, c, out);
        }
    }
}

/// Collect all `~[N]~>` rules in an AST (rules file or the same module).
///
/// `bound` is the set of names bound by the module (assign targets,
/// `op`/`data` names, import-bound names): a free `var` inside a rule
/// is a pattern metavariable, a bound one is a literal reference.
pub fn collect_rules(
    ast: &Ast,
    bound: &HashSet<String>,
    consts: &HashMap<String, u64>,
    lang: &Language,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    for i in 0..ast.len() {
        collect_at(ast, AstRef::new(i), bound, consts, lang, &mut rules);
    }
    rules
}

/// Names bound by a module AST: `assign`-style targets (declared via the
/// `target` + `value` roles), declaration `name` payloads (`op`,
/// `data`, …), and `stmt.import` bound names.
pub fn bound_names(ast: &Ast, lang: &Language) -> HashSet<String> {
    let mut bound = HashSet::new();
    for i in 0..ast.len() {
        let node = match ast.get(AstRef::new(i)) {
            Some(n) => n,
            None => continue,
        };
        if let Some(name) = node.payload.get("name").and_then(|v| v.as_str()) {
            bound.insert(name.to_string());
        }
        if node.kind == "stmt.import" {
            if let Some(a) = node.payload.get("alias").and_then(|v| v.as_str()) {
                if !a.is_empty() {
                    bound.insert(a.to_string());
                }
            }
            match node.payload.get("names") {
                Some(Value::Array(items)) => {
                    for item in items {
                        match item {
                            Value::String(s) if s != "*" => {
                                bound.insert(s.clone());
                            }
                            Value::Object(o) => {
                                if let Some(a) = o.get("as").and_then(|v| v.as_str()) {
                                    bound.insert(a.to_string());
                                } else if let Some(n) = o.get("name").and_then(|v| v.as_str()) {
                                    bound.insert(n.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some(Value::String(s)) if s != "*" => {
                    bound.insert(s.clone());
                }
                _ => {}
            }
            for c in child_refs(node) {
                if let Some(n) = ast.get(c) {
                    if let Some(name) = var_name(n) {
                        bound.insert(name);
                    }
                }
            }
        }
        let has_target = lang
            .child_role_index(&node.kind, "target")
            .ok()
            .flatten()
            .is_some();
        let has_value = lang
            .child_role_index(&node.kind, "value")
            .ok()
            .flatten()
            .is_some();
        if has_target && has_value {
            if let Some(t) = child_ref(node, lang, "target") {
                if let Some(tn) = ast.get(AstRef::new(t)) {
                    if let Some(name) = target_name(ast, lang, tn) {
                        bound.insert(name);
                    }
                    // A generic binding `f<D>` scopes its dimension
                    // parameters over the value — they are bound names,
                    // not pattern metavariables.
                    if let Some(args_pos) = lang.child_role_index(&tn.kind, "args").ok().flatten() {
                        if let Some(children) =
                            tn.payload.get("children").and_then(|c| c.as_array())
                        {
                            for arg in children.iter().skip(args_pos) {
                                if let Some(an) =
                                    arg.as_u64().and_then(|i| ast.get(AstRef::new(i as usize)))
                                {
                                    if let Some(name) = var_name(an) {
                                        bound.insert(name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    bound
}

/// The bound name of a binding target: a bare `var`, or the `target`/
/// `subject`/`body` role child followed recursively (a `dimension`
/// target `name<D>` binds `name`). Role-driven per ADR-0081.
fn target_name(ast: &Ast, lang: &Language, node: &AstNode) -> Option<String> {
    if let Some(name) = var_name(node) {
        return Some(name);
    }
    let children = node.payload.get("children")?.as_array()?;
    for role in ["target", "subject", "body"] {
        if let Some(pos) = lang.child_role_index(&node.kind, role).ok().flatten() {
            if let Some(raw) = children.get(pos).and_then(|v| v.as_u64()) {
                if let Some(child) = ast.get(AstRef::new(raw as usize)) {
                    if let Some(name) = target_name(ast, lang, child) {
                        return Some(name);
                    }
                }
            }
        }
    }
    None
}

fn var_name(node: &AstNode) -> Option<String> {
    if node.kind == "var" {
        node.payload.as_str().map(String::from)
    } else {
        None
    }
}

/// Find the value of the last `<name> := <a> >>> <b> >>> ...` assignment —
/// the 1-cell pipeline the transform should rewrite.
pub fn pipeline_value(ast: &Ast, lang: &Language) -> Option<AstRef> {
    let mut found = None;
    for i in 0..ast.len() {
        if let Some(v) = pipeline_at(ast, AstRef::new(i), lang) {
            found = Some(v);
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Two-file `hnl transform <rules.hnl> <program.hnl>` driver
// ---------------------------------------------------------------------------

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
    let mut bound = HashSet::new();
    let mut rule_env = HashMap::new();
    for (ast, _) in &rules_stmts {
        bound.extend(bound_names(ast, lang));
        rule_env.extend(dim_env(ast, lang));
    }
    let mut rules = Vec::new();
    for (ast, _) in &rules_stmts {
        rules.extend(collect_rules(ast, &bound, &rule_env, lang));
    }
    if rules.is_empty() {
        eprintln!("transform: no ~[N]~> rules found in {rules_path}");
        return 1;
    }

    let prog_stmts = match parse_raw(lang, &prog_src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("transform: {prog_path}: parse error: {e}");
            return 2;
        }
    };

    let mut prog_env = HashMap::new();
    for (ast, _) in &prog_stmts {
        prog_env.extend(dim_env(ast, lang));
    }
    let prog_env = Arc::new(prog_env);

    for (ast, _) in &prog_stmts {
        if let Some(value_ref) = pipeline_value(ast, lang) {
            let protected = Arc::new(dim_arrow_subtrees(ast, lang));
            let engine = RewriteEngine::new().with_native_registry(native(
                rules.clone(),
                protected,
                prog_env.clone(),
            ));
            let out = match engine.run_pass(lang, ast, value_ref, "n_dim_opt") {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("transform: pass failed: {e}");
                    return 1;
                }
            };
            match FormatEngine::new().format(lang, &out.ast, out.root) {
                Ok(s) => {
                    println!("{s}");
                    return 0;
                }
                Err(e) => {
                    eprintln!("transform: emit failed: {e}");
                    return 1;
                }
            }
        }
    }

    eprintln!("transform: no 1-cell pipeline (`<name> := a >>> b`) found in {prog_path}");
    1
}

// ---------------------------------------------------------------------------
// Rule extraction
// ---------------------------------------------------------------------------

fn collect_at(
    ast: &Ast,
    r: AstRef,
    bound: &HashSet<String>,
    consts: &HashMap<String, u64>,
    lang: &Language,
    rules: &mut Vec<Rule>,
) {
    let Some(node) = ast.get(r) else { return };
    if is_arrow_kind(lang, &node.kind) {
        if let (Some(src), Some(dim), Some(tgt)) = (
            child_ref(node, lang, "source"),
            child_ref(node, lang, "dim"),
            child_ref(node, lang, "target"),
        ) {
            if let Some(dim) = rule_dim(ast, AstRef::new(dim), consts) {
                let source = unwrap_paren(ast, AstRef::new(src), lang);
                let target = unwrap_paren(ast, AstRef::new(tgt), lang);
                let mut metavars = HashSet::new();
                collect_metavars(ast, source, bound, &mut metavars);
                collect_metavars(ast, target, bound, &mut metavars);
                rules.push(Rule {
                    source_ast: Arc::new(ast.clone()),
                    source,
                    target,
                    dim,
                    metavars,
                });
            }
        }
    }
    for c in child_refs(node) {
        collect_at(ast, c, bound, consts, lang, rules);
    }
}

/// Free `var` names in a term — the pattern metavariables of a rule.
fn collect_metavars(ast: &Ast, r: AstRef, bound: &HashSet<String>, out: &mut HashSet<String>) {
    let Some(node) = ast.get(r) else { return };
    if let Some(name) = var_name(node) {
        if !bound.contains(&name) {
            out.insert(name);
        }
        return;
    }
    for c in child_refs(node) {
        collect_metavars(ast, c, bound, out);
    }
}

/// The dimension environment of an AST — `name ↦ dimension` — collected
/// so dimension expressions can refer to assigned names, just like any
/// other expression position:
///
/// - `d := 4` — a constant usable in `cert ~[d]~>`.
/// - `cert := A ~[k]~> B` — `cert` is a k-cell wherever it appears.
///
/// Two passes: constants first (a `~[d]~>` dim may itself be a reference),
/// then assigned dim_arrows (resolved through the constants).
pub fn dim_env(ast: &Ast, lang: &Language) -> HashMap<String, u64> {
    let mut env = HashMap::new();
    for pass in 0..2 {
        for i in 0..ast.len() {
            let Some(node) = ast.get(AstRef::new(i)) else {
                continue;
            };
            let has_target = lang
                .child_role_index(&node.kind, "target")
                .ok()
                .flatten()
                .is_some();
            let has_value = lang
                .child_role_index(&node.kind, "value")
                .ok()
                .flatten()
                .is_some();
            if !(has_target && has_value) {
                continue;
            }
            let (Some(t), Some(v)) = (
                child_ref(node, lang, "target"),
                child_ref(node, lang, "value"),
            ) else {
                continue;
            };
            let (Some(tn), Some(vn)) = (ast.get(AstRef::new(t)), ast.get(AstRef::new(v))) else {
                continue;
            };
            let Some(name) = var_name(tn) else { continue };
            if pass == 0 {
                if let Some(k) = number_lit(vn) {
                    env.insert(name, k);
                }
            } else {
                let v = unwrap_paren(ast, AstRef::new(v), lang);
                if let Some(vn) = ast.get(v) {
                    if is_arrow_kind(lang, &vn.kind) {
                        if let Some(dim) = child_ref(vn, lang, "dim") {
                            if let Some(RuleDim::Fixed(k)) = rule_dim(ast, AstRef::new(dim), &env) {
                                env.insert(name, k);
                            }
                        }
                    }
                }
            }
        }
    }
    env
}

fn number_lit(node: &AstNode) -> Option<u64> {
    if node.kind != "value" {
        return None;
    }
    match node.payload.get("value").unwrap_or(&node.payload) {
        Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(|i| i as u64)),
        _ => None,
    }
}

/// The dimension of a `~[dim]~>` slot, which is a normal expression:
/// a number literal is `Fixed(k)`; a `var` resolves through `consts`
/// (assigned constants — `a := 4`) or is `Any` (a generic dimension
/// variable like `N` in `dedupe<N>`, or a metavariable); any other
/// expression (`a >>> b`, `f(x)`) is `Any` — the transform treats an
/// opaque dimension expression as generic.
fn rule_dim(ast: &Ast, r: AstRef, consts: &HashMap<String, u64>) -> Option<RuleDim> {
    let node = ast.get(r)?;
    if let Some(name) = var_name(node) {
        return Some(
            consts
                .get(&name)
                .map(|&k| RuleDim::Fixed(k))
                .unwrap_or(RuleDim::Any),
        );
    }
    if node.kind == "value" {
        return number_lit(node).map(RuleDim::Fixed).or(Some(RuleDim::Any));
    }
    Some(RuleDim::Any)
}

/// The cell dimension of a chain element: a `dim_arrow` literal with a
/// concrete dimension `d` is a d-cell; a `var` resolves through `env`
/// (`cert := A ~[k]~> B` makes `cert` a k-cell); anything else is a
/// 1-cell. A symbolic-dimension cell is `None` (only generic rules can
/// rewrite it).
fn cell_dim(ast: &Ast, r: AstRef, lang: &Language, env: &HashMap<String, u64>) -> Option<u64> {
    let r = unwrap_paren(ast, r, lang);
    let node = ast.get(r)?;
    if is_arrow_kind(lang, &node.kind) {
        let dim = child_ref(node, lang, "dim")?;
        return match rule_dim(ast, AstRef::new(dim), env)? {
            RuleDim::Fixed(k) => Some(k),
            RuleDim::Any => None,
        };
    }
    if let Some(name) = var_name(node) {
        return Some(env.get(&name).copied().unwrap_or(1));
    }
    Some(1)
}

fn unwrap_paren(ast: &Ast, r: AstRef, _lang: &Language) -> AstRef {
    let mut cur = r;
    while let Some(node) = ast.get(cur) {
        if node.kind != "paren" {
            break;
        }
        if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
            if let Some(first) = children.first().and_then(|v| v.as_u64()) {
                cur = AstRef::new(first as usize);
                continue;
            }
        }
        break;
    }
    cur
}

fn pipeline_at(ast: &Ast, r: AstRef, lang: &Language) -> Option<AstRef> {
    let node = ast.get(r)?;
    if node.kind != "assign" {
        return None;
    }
    let value = child_ref(node, lang, "value")?;
    let value_node = ast.get(unwrap_paren(ast, AstRef::new(value), lang))?;
    match value_node.kind.as_str() {
        "flow" => Some(AstRef::new(value)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Native rewrite
// ---------------------------------------------------------------------------

fn n_dim_arrow_opt(
    ctx: &mut RewriteCtx<'_>,
    rules: &Arc<Vec<Rule>>,
    protected: &HashSet<usize>,
    consts: &HashMap<String, u64>,
) -> Result<AstRef, LangError> {
    let node = ctx.node();
    if node.kind != "flow" || protected.contains(&ctx.node_ref().index()) {
        return Ok(ctx.pass_through());
    }

    let lang = ctx.lang();
    let left_role = lang
        .child_role_index(&node.kind, "left")?
        .ok_or_else(|| LangError::engine("flow node missing left role"))?;
    let right_role = lang
        .child_role_index(&node.kind, "right")?
        .ok_or_else(|| LangError::engine("flow node missing right role"))?;

    let left_new = ctx
        .remap_child(left_role)
        .ok_or_else(|| LangError::engine("flow left child not remapped"))?;
    let right_new = ctx
        .remap_child(right_role)
        .ok_or_else(|| LangError::engine("flow right child not remapped"))?;

    let out = ctx.out();
    let mut chain = flatten_flow(out, AstRef::new(left_new), lang);
    chain.extend(flatten_flow(out, AstRef::new(right_new), lang));
    let final_chain = apply_rules_to_chain(rules, out, &chain, lang, consts)?;
    Ok(build_flow(out, &final_chain))
}

fn flatten_flow(ast: &Ast, root: AstRef, lang: &Language) -> Vec<usize> {
    let root = unwrap_paren(ast, root, lang);
    let Some(node) = ast.get(root) else {
        return vec![root.index()];
    };
    if node.kind != "flow" {
        return vec![root.index()];
    }
    if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
        if let (Some(l), Some(r)) = (
            children.get(0).and_then(|v| v.as_u64()),
            children.get(1).and_then(|v| v.as_u64()),
        ) {
            let mut res = flatten_flow(ast, AstRef::new(l as usize), lang);
            res.extend(flatten_flow(ast, AstRef::new(r as usize), lang));
            return res;
        }
    }
    vec![root.index()]
}

fn build_flow(out: &mut Ast, chain: &[usize]) -> AstRef {
    if chain.len() == 1 {
        return AstRef::new(chain[0]);
    }
    let right = build_flow(out, &chain[1..]);
    let payload = json!({
        "children": [chain[0] as u64, right.index() as u64]
    });
    out.push(AstNode::new("flow").with_payload(payload))
}

fn apply_rules_to_chain(
    rules: &Arc<Vec<Rule>>,
    out: &mut Ast,
    chain: &[usize],
    lang: &Language,
    consts: &HashMap<String, u64>,
) -> Result<Vec<usize>, LangError> {
    let mut cur = chain.to_vec();
    let mut changed = true;
    while changed {
        changed = false;
        for rule in rules.iter() {
            let src_flat = flatten_flow(&rule.source_ast, rule.source, lang);
            if src_flat.is_empty() || src_flat.len() > cur.len() {
                continue;
            }
            for start in 0..=cur.len() - src_flat.len() {
                let slice = &cur[start..start + src_flat.len()];
                // A `~[k]~>` rule rewrites (k-1)-cells; a `~[N]~>`
                // (dimension variable) rule fires at every dimension.
                if let RuleDim::Fixed(k) = rule.dim {
                    if cell_dim(out, AstRef::new(slice[0]), lang, consts) != Some(k - 1) {
                        continue;
                    }
                }
                let mut bindings = HashMap::new();
                let mut ok = true;
                for (i, s) in src_flat.iter().enumerate() {
                    if !match_ast(
                        &rule.source_ast,
                        AstRef::new(*s),
                        out,
                        AstRef::new(slice[i]),
                        lang,
                        &rule.metavars,
                        &mut bindings,
                    )? {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    let tgt_ref = copy_subst(&rule.source_ast, rule.target, out, &bindings)?;
                    let tgt_chain = if out.get(tgt_ref).map(|n| n.kind == "flow").unwrap_or(false) {
                        flatten_flow(out, tgt_ref, lang)
                    } else {
                        vec![tgt_ref.index()]
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
    Ok(cur)
}

type Bindings = HashMap<String, AstRef>;

fn match_ast(
    a_ast: &Ast,
    a: AstRef,
    b_ast: &Ast,
    b: AstRef,
    lang: &Language,
    metavars: &HashSet<String>,
    bindings: &mut Bindings,
) -> Result<bool, LangError> {
    // Parens are transparent: `x ~> x` and `(x ~> x)` denote the same term.
    let a = unwrap_paren(a_ast, a, lang);
    let b = unwrap_paren(b_ast, b, lang);
    let an = match a_ast.get(a) {
        Some(n) => n,
        None => return Ok(false),
    };
    // A pattern metavariable binds the matched term on first use and
    // must match the same term structurally on later uses (`f >>> f`
    // requires both sides equal).
    if let Some(name) = var_name(an) {
        if metavars.contains(&name) {
            return match bindings.get(&name) {
                Some(&bound) => match_ast(
                    b_ast,
                    bound,
                    b_ast,
                    b,
                    lang,
                    &HashSet::new(),
                    &mut Bindings::new(),
                ),
                None => {
                    bindings.insert(name, b);
                    Ok(true)
                }
            };
        }
    }
    let bn = match b_ast.get(b) {
        Some(n) => n,
        None => return Ok(false),
    };
    if an.kind != bn.kind {
        return Ok(false);
    }
    Ok(match_payload(
        a_ast,
        &an.payload,
        b_ast,
        &bn.payload,
        lang,
        metavars,
        bindings,
    ))
}

fn match_payload(
    a_ast: &Ast,
    a_payload: &Value,
    b_ast: &Ast,
    b_payload: &Value,
    lang: &Language,
    metavars: &HashSet<String>,
    bindings: &mut Bindings,
) -> bool {
    match (a_payload, b_payload) {
        (Value::Array(a_arr), Value::Array(b_arr)) => {
            if a_arr.len() != b_arr.len() {
                return false;
            }
            for (av, bv) in a_arr.iter().zip(b_arr.iter()) {
                if !match_payload_value(a_ast, av, b_ast, bv, lang, metavars, bindings) {
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
                if !match_object_value(a_ast, k, v, b_ast, bv, lang, metavars, bindings) {
                    return false;
                }
            }
            true
        }
        _ => a_payload == b_payload,
    }
}

fn match_object_value(
    a_ast: &Ast,
    key: &str,
    v: &Value,
    b_ast: &Ast,
    bv: &Value,
    lang: &Language,
    metavars: &HashSet<String>,
    bindings: &mut Bindings,
) -> bool {
    match key {
        "children" => match (v, bv) {
            (Value::Array(a_arr), Value::Array(b_arr)) => {
                if a_arr.len() != b_arr.len() {
                    return false;
                }
                for (av, bvv) in a_arr.iter().zip(b_arr.iter()) {
                    if !match_payload_value(a_ast, av, b_ast, bvv, lang, metavars, bindings) {
                        return false;
                    }
                }
                true
            }
            _ => v == bv,
        },
        "body" => match (v, bv) {
            (Value::Number(a_n), Value::Number(b_n))
                if a_n.as_u64().is_some() && b_n.as_u64().is_some() =>
            {
                let a_i = a_n.as_u64().unwrap() as usize;
                let b_i = b_n.as_u64().unwrap() as usize;
                match_ast(
                    a_ast,
                    AstRef::new(a_i),
                    b_ast,
                    AstRef::new(b_i),
                    lang,
                    metavars,
                    bindings,
                )
                .unwrap_or(false)
            }
            _ => v == bv,
        },
        _ => v == bv,
    }
}

fn match_payload_value(
    a_ast: &Ast,
    v: &Value,
    b_ast: &Ast,
    bv: &Value,
    lang: &Language,
    metavars: &HashSet<String>,
    bindings: &mut Bindings,
) -> bool {
    match (v, bv) {
        (Value::Number(a_n), Value::Number(b_n))
            if a_n.as_u64().is_some() && b_n.as_u64().is_some() =>
        {
            let a_i = a_n.as_u64().unwrap() as usize;
            let b_i = b_n.as_u64().unwrap() as usize;
            match_ast(
                a_ast,
                AstRef::new(a_i),
                b_ast,
                AstRef::new(b_i),
                lang,
                metavars,
                bindings,
            )
            .unwrap_or(false)
        }
        (Value::Array(_), Value::Array(_)) => {
            match_payload(a_ast, v, b_ast, bv, lang, metavars, bindings)
        }
        (Value::Object(_), Value::Object(_)) => {
            match_payload(a_ast, v, b_ast, bv, lang, metavars, bindings)
        }
        _ => v == bv,
    }
}

/// Copy a rule target into `out`, substituting pattern metavariables.
/// `bindings` maps metavar names to refs in `out` itself (the arena the
/// match ran against), so bound terms are copied within the same arena.
fn copy_subst(
    src: &Ast,
    r: AstRef,
    out: &mut Ast,
    bindings: &Bindings,
) -> Result<AstRef, LangError> {
    let node = src
        .get(r)
        .ok_or_else(|| LangError::engine("copy_subst: bad ref"))?;
    if let Some(name) = var_name(node) {
        if let Some(&bound) = bindings.get(&name) {
            return copy_within(bound, out);
        }
    }
    let mut new_node = node.clone();
    if let Some(children) = node
        .payload
        .get("children")
        .and_then(|c| c.as_array())
        .cloned()
    {
        let mut remapped = Vec::with_capacity(children.len());
        for c in children {
            match c.as_u64() {
                Some(idx) => {
                    let copied = copy_subst(src, AstRef::new(idx as usize), out, bindings)?;
                    remapped.push(Value::from(copied.index() as u64));
                }
                None => remapped.push(c),
            }
        }
        new_node
            .payload
            .as_object_mut()
            .map(|o| o.insert("children".to_string(), Value::Array(remapped)));
    }
    for key in ARENA_REF_PAYLOAD_KEYS {
        if let Some(idx) = node.payload.get(*key).and_then(|v| v.as_u64()) {
            let copied = copy_subst(src, AstRef::new(idx as usize), out, bindings)?;
            new_node
                .payload
                .as_object_mut()
                .map(|o| o.insert((*key).to_string(), Value::from(copied.index() as u64)));
        }
    }
    Ok(out.push(new_node))
}

/// Copy a subtree of `out` to fresh nodes within `out` (used to splice a
/// metavariable binding into a rewritten term).
fn copy_within(src_ref: AstRef, out: &mut Ast) -> Result<AstRef, LangError> {
    let sub = out.subtree(src_ref)?;
    let base = out.len() as u64;
    for n in sub.nodes() {
        let mut node = n.clone();
        if let Some(children) = node
            .payload
            .get_mut("children")
            .and_then(|c| c.as_array_mut())
        {
            for c in children.iter_mut() {
                if let Some(idx) = c.as_u64() {
                    *c = Value::from(idx + base);
                }
            }
        }
        for key in ARENA_REF_PAYLOAD_KEYS {
            if let Some(v) = node.payload.get(*key).and_then(|v| v.as_u64()) {
                node.payload
                    .as_object_mut()
                    .unwrap()
                    .insert((*key).to_string(), Value::from(v + base));
            }
        }
        out.push(node);
    }
    Ok(AstRef::new(base as usize + sub.len().saturating_sub(1)))
}

// ---------------------------------------------------------------------------
// Generic child helpers
// ---------------------------------------------------------------------------

fn child_ref(node: &AstNode, lang: &Language, role: &str) -> Option<usize> {
    let pos = lang.child_role_index(&node.kind, role).ok().flatten()?;
    node.payload
        .get("children")?
        .as_array()?
        .get(pos)?
        .as_u64()
        .map(|v| v as usize)
}

fn child_refs(node: &AstNode) -> Vec<AstRef> {
    let mut refs = Vec::new();
    if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
        for v in children {
            if let Some(idx) = v.as_u64() {
                refs.push(AstRef::new(idx as usize));
            }
        }
    }
    if let Some(body) = node.payload.get("body").and_then(|b| b.as_u64()) {
        refs.push(AstRef::new(body as usize));
    }
    refs
}

// ---------------------------------------------------------------------------
// Raw parsing helpers (mirrors main.rs transform_parse_raw)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StatementInfo {
    source: String,
    start: usize,
}

fn parse_raw(lang: &Language, src: &str) -> Result<Vec<(Ast, StatementInfo)>, LangError> {
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

/// Delegates to the shared `plugin-ext-diag` splitter (mirrors
/// `transform_parse_raw` in main.rs).
fn split_statements(src: &str) -> Vec<StatementInfo> {
    handl_plugin_ext_diag::split_statements(src)
        .into_iter()
        .map(|c| StatementInfo {
            source: c.text,
            start: c.offset,
        })
        .collect()
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
