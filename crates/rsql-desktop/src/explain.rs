mod decode;

use serde::Deserialize;
use serde_json::{Number, Value};

#[derive(Debug, Deserialize)]
pub struct ExplainPlan {
    #[serde(rename = "Plan")]
    pub root: PlanNode,
    #[serde(rename = "Planning Time")]
    pub planning_time: Option<f64>,
    #[serde(rename = "Execution Time")]
    pub execution_time: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PlanNode {
    #[serde(rename = "Node Type")]
    node_type: String,
    #[serde(rename = "Join Type")]
    join_type: Option<String>,
    #[serde(rename = "Strategy")]
    strategy: Option<String>,
    #[serde(rename = "Relation Name")]
    relation_name: Option<String>,
    #[serde(rename = "Alias")]
    alias: Option<String>,
    #[serde(rename = "Index Name")]
    index_name: Option<String>,
    #[serde(rename = "Filter")]
    filter: Option<String>,
    #[serde(rename = "Index Cond")]
    index_cond: Option<String>,
    #[serde(rename = "Hash Cond")]
    hash_cond: Option<String>,
    #[serde(rename = "Merge Cond")]
    merge_cond: Option<String>,
    #[serde(default, rename = "Sort Key")]
    sort_key: Vec<String>,
    #[serde(rename = "Actual Total Time")]
    pub actual_total_time: Option<f64>,
    #[serde(rename = "Actual Loops")]
    pub actual_loops: Option<u64>,
    #[serde(rename = "Plan Rows")]
    pub plan_rows: Option<Number>,
    #[serde(rename = "Actual Rows")]
    pub actual_rows: Option<Number>,
    #[serde(rename = "Startup Cost")]
    pub startup_cost: Option<f64>,
    #[serde(rename = "Total Cost")]
    pub total_cost: Option<f64>,
    #[serde(rename = "Shared Hit Blocks")]
    pub hit_blocks: Option<u64>,
    #[serde(rename = "Shared Read Blocks")]
    pub read_blocks: Option<u64>,
    #[serde(default, rename = "Plans")]
    pub children: Vec<PlanNode>,
}

impl Drop for PlanNode {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(mut node) = pending.pop() {
            pending.append(&mut node.children);
        }
    }
}

impl ExplainPlan {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let entry = value.get(0).ok_or("Missing EXPLAIN plan")?;
        Self::deserialize(serde_stacker::Deserializer::new(entry))
            .map_err(|e| format!("Invalid EXPLAIN plan: {e}"))
    }

    pub fn analyzed(&self) -> bool {
        self.root.actual_rows.is_some()
    }

    pub fn percent(&self, node: &PlanNode) -> Option<f64> {
        let (weight, root) = if self.analyzed() {
            (node.elapsed()?, self.root.actual_total_time?)
        } else {
            (node.total_cost?, self.root.total_cost?)
        };
        Some(if root > 0. { weight / root * 100. } else { 0. })
    }
}

impl PlanNode {
    pub fn label(&self) -> String {
        let mut label = match &self.join_type {
            Some(join) => format!("{join} {}", self.node_type),
            None => self.node_type.clone(),
        };
        if let Some(strategy) = &self.strategy {
            label.push_str(&format!(" ({strategy})"));
        }
        if let Some(table) = &self.relation_name {
            label.push_str(&format!(" on {table}"));
            if let Some(alias) = self.alias.as_ref().filter(|a| *a != table) {
                label.push_str(&format!(" as {alias}"));
            }
        }
        if let Some(index) = &self.index_name {
            label.push_str(&format!(" using {index}"));
        }
        label
    }

    pub fn details(&self) -> Vec<String> {
        let mut details = [
            ("Filter", &self.filter),
            ("Index Cond", &self.index_cond),
            ("Hash Cond", &self.hash_cond),
            ("Merge Cond", &self.merge_cond),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.as_ref().map(|v| format!("{name}: {v}")))
        .collect::<Vec<_>>();
        if !self.sort_key.is_empty() {
            details.push(format!("Sort Key: {}", self.sort_key.join(", ")));
        }
        details
    }

    pub fn elapsed(&self) -> Option<f64> {
        Some(self.actual_total_time? * self.actual_loops? as f64)
    }

    pub fn has_zero_estimate(&self) -> bool {
        self.plan_rows.as_ref().and_then(Number::as_f64) == Some(0.)
    }

    pub fn row_ratio(&self) -> Option<f64> {
        let estimate = self.plan_rows.as_ref()?.as_f64().filter(|n| *n > 0.)?;
        Some(self.actual_rows.as_ref()?.as_f64()? / estimate)
    }

    pub fn row_warning(&self) -> bool {
        self.row_ratio().is_some_and(|r| !(0.1..=10.).contains(&r))
    }
}

#[derive(Debug)]
pub struct PlanOutput {
    pub raw: String,
    pub plan: ExplainPlan,
}
