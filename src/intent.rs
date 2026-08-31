//! Edition 4 typed intent inspection.
//!
//! The graph is deliberately a compiler-side value. Pure pipelines are
//! lowered directly to ordinary bytecode and therefore allocate no runtime
//! plan. Only `prepare` creates an explicitly stored, immutable plan value;
//! its payload continues to use the checked record representation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ast::{Expr, Span, Stmt};

pub const SCHEMA: &str = "org.nivren.intent.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optimization {
    Disabled,
    Enabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentGraph {
    pub schema: String,
    pub target: String,
    pub optimized: bool,
    pub summary: IntentSummary,
    pub nodes: Vec<IntentNode>,
    #[serde(skip)]
    plan_payloads: BTreeMap<usize, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentSummary {
    pub required_capabilities: Vec<String>,
    pub resources: Vec<String>,
    pub effect_count: usize,
    pub materialized_plans: usize,
    pub pure_runtime_plan_allocations: usize,
    pub fused_pipelines: usize,
    pub serial_effect_order: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentNode {
    pub id: usize,
    pub kind: String,
    pub operation: String,
    pub line: usize,
    pub column: usize,
    pub allocation: String,
    pub capabilities: Vec<String>,
    pub resources: Vec<String>,
    pub cancellation: String,
    pub retries: String,
    pub timeout: String,
    pub buffering: String,
    pub blocking: String,
    pub target: String,
    pub fusion: String,
    pub effect_order: Option<usize>,
    pub serializable: bool,
    pub portability: String,
}

impl IntentGraph {
    /// Stable, human-reviewable output used by `niv explain` and snapshots.
    pub fn to_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .expect("the versioned intent graph contains only serializable values");
        output.push('\n');
        output
    }

    /// Deterministic plain-language rendering of the same validated graph:
    /// equal graphs produce equal stories, and every sentence comes from
    /// graph data rather than heuristics.
    pub fn story(&self) -> String {
        if self.nodes.is_empty() {
            return "This program declares no external intent: it is pure computation.\n".into();
        }
        let mut output = String::from("This program's intent, in source order:\n");
        for node in &self.nodes {
            let line = node.line;
            let sentence = match node.kind.as_str() {
                "effect" => {
                    let verb = effect_verb(&node.capabilities);
                    let needs = if node.capabilities.is_empty() {
                        String::new()
                    } else {
                        format!(" (needs {})", node.capabilities.join(", "))
                    };
                    let order = node
                        .effect_order
                        .map(|order| format!(" as effect {order}"))
                        .unwrap_or_default();
                    format!(
                        "- It {verb} {}{needs}{order} at line {line}.",
                        node.operation
                    )
                }
                "prepared-plan" => {
                    let portability = if node.serializable {
                        "the plan is portable data"
                    } else {
                        "the plan stays local because it holds authority, a handle, a callback, a secret, or an effect"
                    };
                    format!(
                        "- It prepares one immutable {} plan at line {line}; {portability}.",
                        node.operation
                    )
                }
                "pipeline" => {
                    let fusion = match node.fusion.as_str() {
                        "verified-pure" => "pure and fused",
                        "disabled" => "pure with fusion disabled",
                        _ => "with effect stages kept in source order",
                    };
                    format!(
                        "- It flows values through {} at line {line}, {fusion}.",
                        node.operation
                    )
                }
                "perform-boundary" => {
                    format!("- It marks a visible effect boundary at line {line}.")
                }
                other => format!(
                    "- It records {other} intent for {} at line {line}.",
                    node.operation
                ),
            };
            output.push_str(&sentence);
            output.push('\n');
        }
        let summary = &self.summary;
        let capabilities = if summary.required_capabilities.is_empty() {
            "none".to_string()
        } else {
            summary.required_capabilities.join(", ")
        };
        output.push_str(&format!(
            "Altogether: {} effect(s), {} materialized plan(s), {} fused pipeline(s); required capabilities: {capabilities}.\n",
            summary.effect_count, summary.materialized_plans, summary.fused_pipelines,
        ));
        output
    }

    /// Rejects malformed graphs before they can reach optimization or tooling.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("unsupported intent schema '{}'", self.schema));
        }
        for (expected, node) in self.nodes.iter().enumerate() {
            if node.id != expected {
                return Err("intent node identifiers are not canonical".into());
            }
            if node.capabilities.windows(2).any(|pair| pair[0] >= pair[1])
                || node.resources.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(format!("intent node {} metadata is not canonical", node.id));
            }
        }
        let orders = self
            .nodes
            .iter()
            .filter_map(|node| node.effect_order)
            .collect::<Vec<_>>();
        if orders != (0..orders.len()).collect::<Vec<_>>() {
            return Err("effect ordering contains a gap or reordering".into());
        }
        if self.summary.pure_runtime_plan_allocations != 0 {
            return Err("pure intent must not allocate runtime plans".into());
        }
        if let Some(node) = self
            .nodes
            .iter()
            .find(|node| node.portability == "effect-call-requires-perform")
        {
            return Err(format!(
                "effect '{}' at {}:{} must be inside a visible perform boundary",
                node.operation, node.line, node.column
            ));
        }
        Ok(())
    }

    /// Serializes an explicitly prepared portable data plan. Executable
    /// handles, callbacks, secrets, local authority, effects, and expressions
    /// whose value is not statically self-contained are refused.
    pub fn serialize_plan(&self, node_id: usize) -> Result<String, String> {
        let node = self
            .nodes
            .get(node_id)
            .ok_or_else(|| format!("intent node {node_id} does not exist"))?;
        if node.kind != "prepared-plan" || !node.serializable {
            return Err(format!(
                "intent node {node_id} is not an explicitly portable prepared plan"
            ));
        }
        let payload = self
            .plan_payloads
            .get(&node_id)
            .ok_or_else(|| format!("intent node {node_id} has no portable data payload"))?;
        let mut output = serde_json::to_string_pretty(&serde_json::json!({
            "schema": "org.nivren.portable-plan.v1",
            "type": node.operation,
            "payload": payload,
        }))
        .expect("portable plan envelopes contain only JSON values");
        output.push('\n');
        Ok(output)
    }
}

#[must_use]
pub fn analyze(program: &[Stmt], optimization: Optimization) -> IntentGraph {
    let mut analyzer = Analyzer::new(optimization);
    analyzer.statements(program);
    analyzer.finish()
}

struct Analyzer {
    optimization: Optimization,
    standard_effects: BTreeMap<String, Vec<String>>,
    functions: BTreeMap<String, Vec<String>>,
    nodes: Vec<IntentNode>,
    effect_order: usize,
    plan_payloads: BTreeMap<usize, serde_json::Value>,
}

impl Analyzer {
    fn new(optimization: Optimization) -> Self {
        Self {
            optimization,
            standard_effects: crate::typecheck::standard_effects(),
            functions: BTreeMap::new(),
            nodes: Vec::new(),
            effect_order: 0,
            plan_payloads: BTreeMap::new(),
        }
    }

    fn finish(self) -> IntentGraph {
        let mut capabilities = BTreeSet::new();
        let mut resources = BTreeSet::new();
        let mut effect_count = 0;
        let mut materialized_plans = 0;
        let mut fused_pipelines = 0;
        let mut serial_effect_order = Vec::new();
        for node in &self.nodes {
            capabilities.extend(node.capabilities.iter().cloned());
            resources.extend(node.resources.iter().cloned());
            if let Some(order) = node.effect_order {
                effect_count += 1;
                serial_effect_order.push(order);
            }
            materialized_plans += usize::from(node.kind == "prepared-plan");
            fused_pipelines += usize::from(node.fusion == "verified-pure");
        }
        IntentGraph {
            schema: SCHEMA.into(),
            target: "vm-bytecode".into(),
            optimized: self.optimization == Optimization::Enabled,
            summary: IntentSummary {
                required_capabilities: capabilities.into_iter().collect(),
                resources: resources.into_iter().collect(),
                effect_count,
                materialized_plans,
                pure_runtime_plan_allocations: 0,
                fused_pipelines,
                serial_effect_order,
            },
            nodes: self.nodes,
            plan_payloads: self.plan_payloads,
        }
    }

    fn statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            if let Stmt::Function { name, needs, .. } = statement {
                self.functions.insert(name.clone(), sorted(needs.clone()));
            }
        }
        for statement in statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Doc { .. } => {}
            Stmt::Prepare {
                plan_type,
                initializer,
                span,
                ..
            } => {
                let capabilities = self.expression_capabilities(initializer);
                let resources = resources_for(&capabilities, Some(plan_type));
                let payload = portable_payload(initializer);
                let portable = capabilities.is_empty()
                    && portable_expression(initializer)
                    && payload.is_some();
                let node_id = self.nodes.len();
                self.push(IntentNode {
                    id: 0,
                    kind: "prepared-plan".into(),
                    operation: plan_type.clone(),
                    line: span.line,
                    column: span.column,
                    allocation: "materialized-immutable".into(),
                    capabilities,
                    resources,
                    cancellation: "declared-at-perform".into(),
                    retries: "none".into(),
                    timeout: "explicit-fields-only".into(),
                    buffering: "bounded-by-fields".into(),
                    blocking: "deferred".into(),
                    target: "vm-bytecode".into(),
                    fusion: "not-applicable".into(),
                    effect_order: None,
                    serializable: portable,
                    portability: if portable {
                        "portable-data-only".into()
                    } else {
                        "contains-authority-handle-callback-secret-or-effect".into()
                    },
                });
                if let Some(payload) = payload.filter(|_| portable) {
                    self.plan_payloads.insert(node_id, payload);
                }
                self.expression(initializer, false);
            }
            Stmt::Let { initializer, .. } | Stmt::LetPattern { initializer, .. } => {
                self.expression(initializer, false)
            }
            Stmt::Expression(expression) | Stmt::Print(expression, _) => {
                self.expression(expression, false);
            }
            Stmt::Block(body, _) | Stmt::Module { body, .. } => self.statements(body),
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition, false);
                self.statement(then_branch);
                if let Some(branch) = else_branch {
                    self.statement(branch);
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.expression(condition, false);
                self.statement(body);
            }
            Stmt::Stop(_)
            | Stmt::Skip(_)
            | Stmt::Promise { .. }
            | Stmt::Trusted { .. }
            | Stmt::Sample { .. }
            | Stmt::Generator { .. }
            | Stmt::Expand { .. } => {}
            Stmt::IfCarries {
                subject,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(subject, false);
                self.statement(then_branch);
                if let Some(branch) = else_branch {
                    self.statement(branch);
                }
            }
            Stmt::For { iterable, body, .. } => {
                self.expression(iterable, false);
                self.statement(body);
            }
            Stmt::Using { resource, body, .. } => {
                self.expression(resource, false);
                self.statement(body);
            }
            Stmt::Function { body, .. } => self.statements(body),
            Stmt::Return(Some(value), _) => self.expression(value, false),
            Stmt::Return(None, _)
            | Stmt::Record { .. }
            | Stmt::Enum { .. }
            | Stmt::Protocol { .. }
            | Stmt::Adoption { .. }
            | Stmt::Import { .. }
            | Stmt::Export { .. } => {}
        }
    }

    fn expression(&mut self, expression: &Expr, within_perform: bool) {
        match expression {
            Expr::Perform(value, span) => {
                let capabilities = self.expression_capabilities(value);
                self.push(boundary_node(*span, capabilities));
                self.expression(value, true);
            }
            Expr::Text(pieces, _) => {
                for piece in pieces {
                    if let crate::ast::TextPiece::Hole(hole) = piece {
                        self.expression(hole, within_perform);
                    }
                }
            }
            Expr::Through(input, stage, span) => {
                let capabilities = self.expression_capabilities(expression);
                let pure = capabilities.is_empty();
                self.push(IntentNode {
                    id: 0,
                    kind: "pipeline".into(),
                    operation: callable_path(stage).unwrap_or_else(|| "pipeline-stage".into()),
                    line: span.line,
                    column: span.column,
                    allocation: "none".into(),
                    capabilities: capabilities.clone(),
                    resources: resources_for(&capabilities, None),
                    cancellation: if pure { "not-applicable" } else { "propagated" }.into(),
                    retries: "stage-defined".into(),
                    timeout: "stage-defined".into(),
                    buffering: if pure { "none" } else { "bounded" }.into(),
                    blocking: if pure { "never" } else { "reported-by-stage" }.into(),
                    target: "vm-bytecode".into(),
                    fusion: if pure && self.optimization == Optimization::Enabled {
                        "verified-pure"
                    } else if pure {
                        "disabled"
                    } else {
                        "serial-effects"
                    }
                    .into(),
                    effect_order: None,
                    serializable: pure && portable_expression(expression),
                    portability: if pure {
                        "pure-values"
                    } else {
                        "effectful-stage"
                    }
                    .into(),
                });
                self.expression(input, within_perform);
                self.expression(stage, within_perform);
            }
            Expr::Call(callee, arguments, _, span) => {
                let path = callable_path(callee).unwrap_or_else(|| "dynamic-call".into());
                let capabilities = self.call_capabilities(&path);
                if !capabilities.is_empty() {
                    let resources = resources_for(&capabilities, Some(&path));
                    let blocking = blocking_for(&capabilities);
                    let effect_order = self.next_effect_order();
                    self.push(IntentNode {
                        id: 0,
                        kind: "effect".into(),
                        operation: path,
                        line: span.line,
                        column: span.column,
                        allocation: "none".into(),
                        capabilities,
                        resources,
                        cancellation: "propagated".into(),
                        retries: "explicit-only".into(),
                        timeout: "explicit-or-package-default".into(),
                        buffering: "bounded".into(),
                        blocking,
                        target: "vm-bytecode".into(),
                        fusion: "ordered".into(),
                        effect_order: Some(effect_order),
                        serializable: false,
                        portability: if within_perform {
                            "executed-at-visible-boundary"
                        } else {
                            "effect-call-requires-perform"
                        }
                        .into(),
                    });
                } else if path.contains("transactions") || path.contains("database") {
                    self.push(IntentNode {
                        id: 0,
                        kind: "resource".into(),
                        operation: path.clone(),
                        line: span.line,
                        column: span.column,
                        allocation: "managed-resource".into(),
                        capabilities: vec![],
                        resources: resources_for(&[], Some(&path)),
                        cancellation: "cleanup-on-scope-exit".into(),
                        retries: "explicit-only".into(),
                        timeout: "operation-defined".into(),
                        buffering: "bounded".into(),
                        blocking: "never-for-in-memory-transaction".into(),
                        target: "vm-bytecode".into(),
                        fusion: "not-applicable".into(),
                        effect_order: None,
                        serializable: false,
                        portability: "managed-transaction-handle".into(),
                    });
                }
                self.expression(callee, within_perform);
                for argument in arguments {
                    self.expression(argument, within_perform);
                }
            }
            Expr::Assign(_, value, _)
            | Expr::Unary(_, value, _)
            | Expr::Propagate(value, _)
            | Expr::Get(value, _, _) => self.expression(value, within_perform),
            Expr::Binary(left, _, right, _)
            | Expr::Logical(left, _, right, _)
            | Expr::Coalesce(left, right, _)
            | Expr::Index(left, right, _) => {
                self.expression(left, within_perform);
                self.expression(right, within_perform);
            }
            Expr::Array(values, _) => {
                for value in values {
                    self.expression(value, within_perform);
                }
            }
            Expr::Match(subject, arms, _) => {
                self.expression(subject, within_perform);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expression(guard, within_perform);
                    }
                    self.expression(&arm.value, within_perform);
                }
            }
            Expr::Literal(_, _) | Expr::Variable(_, _) => {}
        }
    }

    fn expression_capabilities(&self, expression: &Expr) -> Vec<String> {
        let mut capabilities = BTreeSet::new();
        collect_capabilities(
            expression,
            &self.standard_effects,
            &self.functions,
            &mut capabilities,
        );
        capabilities.into_iter().collect()
    }

    fn call_capabilities(&self, path: &str) -> Vec<String> {
        sorted(
            self.standard_effects
                .get(path)
                .or_else(|| self.functions.get(path))
                .cloned()
                .unwrap_or_default(),
        )
    }

    fn next_effect_order(&mut self) -> usize {
        let order = self.effect_order;
        self.effect_order += 1;
        order
    }

    fn push(&mut self, mut node: IntentNode) {
        node.id = self.nodes.len();
        node.capabilities.sort();
        node.capabilities.dedup();
        node.resources.sort();
        node.resources.dedup();
        self.nodes.push(node);
    }
}

fn effect_verb(capabilities: &[String]) -> &'static str {
    for capability in capabilities {
        match capability.as_str() {
            "FileRead" => return "reads through",
            "FileWrite" => return "writes through",
            "Network" => return "talks to the network through",
            "Process" => return "runs a process through",
            "Time" => return "reads or waits on the clock through",
            "Random" => return "draws randomness through",
            "Environment" => return "reads the environment through",
            "Log" => return "logs through",
            "Native" => return "crosses the native boundary through",
            _ => {}
        }
    }
    "uses"
}

fn boundary_node(span: Span, capabilities: Vec<String>) -> IntentNode {
    IntentNode {
        id: 0,
        kind: "perform-boundary".into(),
        operation: "perform".into(),
        line: span.line,
        column: span.column,
        allocation: "none".into(),
        resources: resources_for(&capabilities, None),
        capabilities,
        cancellation: "propagated".into(),
        retries: "plan-defined".into(),
        timeout: "plan-defined".into(),
        buffering: "plan-defined-bounded".into(),
        blocking: "reported-by-child-effects".into(),
        target: "vm-bytecode".into(),
        fusion: "boundary-preserved".into(),
        effect_order: None,
        serializable: false,
        portability: "execution-boundary".into(),
    }
}

fn collect_capabilities(
    expression: &Expr,
    standard: &BTreeMap<String, Vec<String>>,
    functions: &BTreeMap<String, Vec<String>>,
    output: &mut BTreeSet<String>,
) {
    match expression {
        Expr::Call(callee, arguments, _, _) => {
            if let Some(path) = callable_path(callee)
                && let Some(found) = standard.get(&path).or_else(|| functions.get(&path))
            {
                output.extend(found.iter().cloned());
            }
            collect_capabilities(callee, standard, functions, output);
            for argument in arguments {
                collect_capabilities(argument, standard, functions, output);
            }
        }
        Expr::Assign(_, value, _)
        | Expr::Unary(_, value, _)
        | Expr::Propagate(value, _)
        | Expr::Perform(value, _)
        | Expr::Get(value, _, _) => collect_capabilities(value, standard, functions, output),
        Expr::Binary(left, _, right, _)
        | Expr::Logical(left, _, right, _)
        | Expr::Coalesce(left, right, _)
        | Expr::Index(left, right, _)
        | Expr::Through(left, right, _) => {
            collect_capabilities(left, standard, functions, output);
            collect_capabilities(right, standard, functions, output);
        }
        Expr::Array(values, _) => {
            for value in values {
                collect_capabilities(value, standard, functions, output);
            }
        }
        Expr::Match(subject, arms, _) => {
            collect_capabilities(subject, standard, functions, output);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_capabilities(guard, standard, functions, output);
                }
                collect_capabilities(&arm.value, standard, functions, output);
            }
        }
        Expr::Text(pieces, _) => {
            for piece in pieces {
                if let crate::ast::TextPiece::Hole(hole) = piece {
                    collect_capabilities(hole, standard, functions, output);
                }
            }
        }
        Expr::Literal(_, _) | Expr::Variable(_, _) => {}
    }
}

fn callable_path(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Variable(name, _) => Some(name.clone()),
        Expr::Get(parent, name, _) => Some(format!("{}.{}", callable_path(parent)?, name)),
        Expr::Call(callee, _, _, _) => callable_path(callee),
        _ => None,
    }
}

fn resources_for(capabilities: &[String], operation: Option<&str>) -> Vec<String> {
    let mut resources = BTreeSet::new();
    for capability in capabilities {
        let resource = match capability.as_str() {
            "FileRead" | "FileWrite" => "filesystem",
            "Network" => "network-socket",
            "Database" => "database-connection",
            "Process" => "child-process",
            "Environment" => "process-environment",
            "Time" => "clock-or-timer",
            "Random" => "secure-randomness",
            "Native" => "foreign-handle",
            "Task" => "async-task",
            other => other,
        };
        resources.insert(resource.to_string());
    }
    if let Some(operation) = operation {
        if operation.contains("channel") {
            resources.insert("bounded-channel".into());
        }
        if operation.contains("transaction") || operation.contains("database") {
            resources.insert("database-transaction".into());
        }
        if operation.contains("http") || operation.contains("websocket") {
            resources.insert("network-stream".into());
        }
    }
    resources.into_iter().collect()
}

fn blocking_for(capabilities: &[String]) -> String {
    if capabilities
        .iter()
        .any(|capability| matches!(capability.as_str(), "FileRead" | "FileWrite" | "Process"))
    {
        "isolated-blocking-worker".into()
    } else {
        "event-loop-nonblocking".into()
    }
}

fn portable_expression(expression: &Expr) -> bool {
    match expression {
        Expr::Literal(_, _) => true,
        Expr::Variable(name, _) => !non_portable_name(name),
        Expr::Assign(_, _, _) | Expr::Perform(_, _) => false,
        Expr::Unary(_, value, _) | Expr::Propagate(value, _) | Expr::Get(value, _, _) => {
            portable_expression(value)
        }
        Expr::Binary(left, _, right, _)
        | Expr::Logical(left, _, right, _)
        | Expr::Coalesce(left, right, _)
        | Expr::Index(left, right, _)
        | Expr::Through(left, right, _) => portable_expression(left) && portable_expression(right),
        Expr::Call(callee, arguments, _, _) => {
            callable_path(callee).is_some_and(|path| !non_portable_name(&path))
                && arguments.iter().all(portable_expression)
        }
        Expr::Array(values, _) => values.iter().all(portable_expression),
        Expr::Match(subject, arms, _) => {
            portable_expression(subject)
                && arms.iter().all(|arm| {
                    portable_expression(&arm.value)
                        && arm.guard.as_ref().is_none_or(portable_expression)
                })
        }
        Expr::Text(pieces, _) => pieces.iter().all(|piece| match piece {
            crate::ast::TextPiece::Literal(_) => true,
            crate::ast::TextPiece::Hole(hole) => portable_expression(hole),
        }),
    }
}

fn portable_payload(expression: &Expr) -> Option<serde_json::Value> {
    match expression {
        Expr::Literal(crate::ast::Literal::Int(value), _) => Some((*value).into()),
        Expr::Literal(crate::ast::Literal::Float(value), _) => {
            serde_json::Number::from_f64(*value).map(Into::into)
        }
        Expr::Literal(crate::ast::Literal::String(value), _) => Some(value.clone().into()),
        Expr::Literal(crate::ast::Literal::Bool(value), _) => Some((*value).into()),
        Expr::Literal(crate::ast::Literal::Null, _) => Some(serde_json::Value::Null),
        Expr::Array(values, _) => values
            .iter()
            .map(portable_payload)
            .collect::<Option<Vec<_>>>()
            .map(Into::into),
        Expr::Call(_, arguments, Some(labels), _) if arguments.len() == labels.len() => {
            let mut object = serde_json::Map::new();
            for (label, argument) in labels.iter().zip(arguments) {
                object.insert(label.clone(), portable_payload(argument)?);
            }
            Some(object.into())
        }
        _ => None,
    }
}

fn non_portable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "secret",
        "handle",
        "callback",
        "library",
        "file",
        "socket",
        "stream",
        "authority",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::{Optimization, analyze};

    fn program(source: &str) -> Vec<crate::ast::Stmt> {
        let tokens = crate::lexer::scan(source).unwrap();
        let program = crate::parser::parse(tokens).unwrap();
        crate::typecheck::check(&program).unwrap();
        program
    }

    #[test]
    fn pure_pipeline_fuses_without_a_plan_allocation() {
        let graph = analyze(
            &program(
                "define double takes { value is Int } gives Int { give value * 2 }\n21 through double",
            ),
            Optimization::Enabled,
        );
        graph.validate().unwrap();
        assert_eq!(graph.summary.pure_runtime_plan_allocations, 0);
        assert_eq!(graph.summary.fused_pipelines, 1);
        assert_eq!(graph.summary.effect_count, 0);
    }

    #[test]
    fn explain_is_deterministic_and_orders_effects() {
        let source = "define read_both takes { left is String right is String } gives String or String needs FileRead { keep first set perform std.files.read with { path set left } or give\nkeep second set perform std.files.read with { path set right } or give\ngive ok(first + second) }";
        let program = program(source);
        let first = analyze(&program, Optimization::Enabled);
        let second = analyze(&program, Optimization::Enabled);
        assert_eq!(first.to_json(), second.to_json());
        assert_eq!(first.summary.serial_effect_order, vec![0, 1]);
        assert_eq!(first.summary.required_capabilities, vec!["FileRead"]);
        first.validate().unwrap();
    }

    #[test]
    fn prepared_plan_reports_portability_and_materialization() {
        let graph = analyze(
            &program(
                "shape Request holds { path is String }\nprepare request as Request with { path set \"/health\" }\n(perform request).path",
            ),
            Optimization::Enabled,
        );
        assert_eq!(graph.summary.materialized_plans, 1);
        assert!(graph.nodes[0].serializable);
        assert_eq!(graph.nodes[0].allocation, "materialized-immutable");
        let serialized = graph.serialize_plan(0).unwrap();
        assert!(serialized.contains("org.nivren.portable-plan.v1"));
        assert!(serialized.contains("/health"));
    }

    #[test]
    fn plan_serialization_rejects_nonportable_values() {
        let graph = analyze(
            &program(
                "shape CallbackPlan holds { callback is String }\nkeep callback set \"local\"\nprepare plan as CallbackPlan with { callback set callback }\nperform plan",
            ),
            Optimization::Enabled,
        );
        assert!(!graph.nodes[0].serializable);
        assert!(graph.serialize_plan(0).is_err());
    }
}
