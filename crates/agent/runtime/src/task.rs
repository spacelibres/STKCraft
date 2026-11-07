//! A deterministic task graph over the engine's commands.
//!
//! This is the executor's skeleton, not a model scheduler. A task names a command and its
//! arguments; the graph resolves dependencies, runs them in order and classifies each outcome. The
//! model may propose a graph later, but it cannot execute one outside these checks.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use stkcraft_astro_engine::{Args, CommandOutcome, Registry, Scenario};

/// The largest graph this executor accepts.
const MAX_TASKS: usize = 64;
/// The largest dependency chain this executor accepts.
const MAX_DEPTH: usize = 16;

/// One command in a graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Stable id within the graph.
    pub id: String,
    /// The command id, e.g. `access.compute`.
    pub command: String,
    /// The command's parameters, in the registry's JSON shape.
    #[serde(default)]
    pub arguments: Value,
    /// Tasks that must complete before this one runs.
    #[serde(default, rename = "depends-on", alias = "depends_on")]
    pub depends_on: Vec<String>,
}

/// A directed acyclic graph of tasks.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TaskGraph {
    /// The tasks, in declaration order.
    pub tasks: Vec<TaskSpec>,
}

/// What happened to one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// The command ran.
    Completed,
    /// The scenario lacks what the command needs, or the command is disabled.
    NeedsInput,
    /// A command returned an error.
    Failed,
    /// A dependency did not complete.
    Blocked,
    /// The caller cancelled before this task ran.
    Cancelled,
    /// The task or failure budget stopped the graph before this task ran.
    BudgetExceeded,
}

/// Limits for one graph execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBudget {
    /// The most tasks that may run.
    pub max_tasks: usize,
    /// The most non-completed outcomes allowed before the graph stops.
    pub max_failures: usize,
}

impl TaskBudget {
    /// Build a budget, rejecting zero or out-of-range values.
    ///
    /// # Errors
    ///
    /// Returns a message when `max_tasks` is zero, above the graph limit, or `max_failures` is zero.
    pub fn new(max_tasks: usize, max_failures: usize) -> Result<Self, String> {
        if max_tasks == 0 || max_tasks > MAX_TASKS {
            return Err(format!("max_tasks must be between 1 and {MAX_TASKS}"));
        }
        if max_failures == 0 {
            return Err("max_failures must be at least 1".to_owned());
        }
        Ok(Self {
            max_tasks,
            max_failures,
        })
    }
}

impl Default for TaskBudget {
    fn default() -> Self {
        Self {
            max_tasks: MAX_TASKS,
            max_failures: 3,
        }
    }
}

/// One task's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskOutcome {
    /// The task id.
    pub id: String,
    /// What happened.
    pub status: TaskStatus,
    /// The command's summary, when it ran.
    pub summary: String,
    /// The command's error, when it failed or was blocked.
    pub error: Option<String>,
    /// The command's structured result, when it ran.
    pub result: Option<Value>,
}

impl TaskGraph {
    /// Validate the graph and return a deterministic execution order.
    ///
    /// # Errors
    ///
    /// Returns every structural error: an empty graph, a duplicate id, an unknown command, a
    /// missing dependency, a self-dependency, too many tasks, a chain that is too deep or a cycle.
    pub fn validate(&self, registry: &Registry) -> Result<Vec<String>, Vec<String>> {
        let mut errors = Vec::new();
        if self.tasks.is_empty() {
            errors.push("a task graph needs at least one task".to_owned());
        }
        if self.tasks.len() > MAX_TASKS {
            errors.push(format!(
                "a task graph is limited to {MAX_TASKS} tasks; this one has {}",
                self.tasks.len()
            ));
        }
        let mut by_id = BTreeMap::new();
        for task in &self.tasks {
            if task.id.trim().is_empty() {
                errors.push("a task has an empty id".to_owned());
            } else if by_id.insert(task.id.clone(), task).is_some() {
                errors.push(format!("duplicate task id `{}`", task.id));
            }
            if registry.get(&task.command).is_none() {
                errors.push(format!(
                    "task `{}` names `{}`, which is not in the command registry",
                    task.id, task.command
                ));
            }
        }
        let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for task in &self.tasks {
            indegree.entry(task.id.as_str()).or_insert(0);
            for dependency in &task.depends_on {
                if dependency == &task.id {
                    errors.push(format!("task `{}` depends on itself", task.id));
                    continue;
                }
                if !by_id.contains_key(dependency) {
                    errors.push(format!(
                        "task `{}` depends on `{dependency}`, which is not in the graph",
                        task.id
                    ));
                    continue;
                }
                *indegree.entry(task.id.as_str()).or_insert(0) += 1;
                children
                    .entry(dependency.as_str())
                    .or_default()
                    .push(task.id.as_str());
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut queue: VecDeque<&str> = indegree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
            .collect();
        let mut order = Vec::new();
        let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
        while let Some(id) = queue.pop_front() {
            order.push(id.to_owned());
            let next_depth = depth.get(id).copied().unwrap_or(0) + 1;
            if next_depth > MAX_DEPTH {
                errors.push(format!(
                    "the dependency chain through `{id}` is deeper than {MAX_DEPTH}"
                ));
                break;
            }
            if let Some(children) = children.get(id) {
                for child in children {
                    depth
                        .entry(child)
                        .and_modify(|current| *current = (*current).max(next_depth))
                        .or_insert(next_depth);
                    if let Some(degree) = indegree.get_mut(child) {
                        *degree = degree.saturating_sub(1);
                        if *degree == 0 {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
        if order.len() != self.tasks.len() {
            let ordered: BTreeSet<&str> = order.iter().map(String::as_str).collect();
            let remaining: Vec<&str> = self
                .tasks
                .iter()
                .map(|task| task.id.as_str())
                .filter(|id| !ordered.contains(id))
                .collect();
            errors.push(format!("dependency cycle among: {}", remaining.join(", ")));
        }
        if errors.is_empty() {
            Ok(order)
        } else {
            Err(errors)
        }
    }

    /// Run the graph against one scenario.
    ///
    /// # Errors
    ///
    /// Returns the validation errors before any command runs. A command's own failure is a
    /// [`TaskStatus::Failed`] outcome, not a graph error.
    pub fn execute(
        &self,
        registry: &Registry,
        scenario: &mut Scenario,
    ) -> Result<Vec<TaskOutcome>, Vec<String>> {
        self.execute_with_budget(registry, scenario, &TaskBudget::default())
    }

    /// Run the graph with an explicit task and failure budget.
    ///
    /// # Errors
    ///
    /// As [`Self::execute`], before any command runs.
    pub fn execute_with_budget(
        &self,
        registry: &Registry,
        scenario: &mut Scenario,
        budget: &TaskBudget,
    ) -> Result<Vec<TaskOutcome>, Vec<String>> {
        self.run(
            registry,
            scenario,
            budget,
            &crate::RequestControl::new(crate::CancellationToken::default()),
        )
    }

    /// Run the graph with cooperative cancellation between tasks.
    ///
    /// A cancelled graph keeps the outcomes of tasks that already ran and records a `cancelled`
    /// outcome for the task that would have run next. Commands already executing finish
    /// synchronously; the check is at the task boundary, not inside a command.
    ///
    /// # Errors
    ///
    /// As [`Self::execute`], before any command runs.
    pub fn execute_controlled(
        &self,
        registry: &Registry,
        scenario: &mut Scenario,
        control: &crate::RequestControl,
    ) -> Result<Vec<TaskOutcome>, Vec<String>> {
        self.run(registry, scenario, &TaskBudget::default(), control)
    }

    fn run(
        &self,
        registry: &Registry,
        scenario: &mut Scenario,
        budget: &TaskBudget,
        control: &crate::RequestControl,
    ) -> Result<Vec<TaskOutcome>, Vec<String>> {
        let order = self.validate(registry)?;
        let by_id: BTreeMap<&str, &TaskSpec> = self
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect();
        let mut outcomes: Vec<TaskOutcome> = Vec::new();
        let mut statuses: BTreeMap<String, TaskStatus> = BTreeMap::new();
        for id in order {
            let Some(task) = by_id.get(id.as_str()).copied() else {
                continue;
            };
            let failures = outcomes
                .iter()
                .filter(|outcome| outcome.status != TaskStatus::Completed)
                .count();
            if outcomes.len() >= budget.max_tasks || failures >= budget.max_failures {
                outcomes.push(TaskOutcome {
                    id: task.id.clone(),
                    status: TaskStatus::BudgetExceeded,
                    summary: String::new(),
                    error: Some(format!(
                        "budget stopped the graph ({} task(s), {failures} failure(s))",
                        outcomes.len()
                    )),
                    result: None,
                });
                break;
            }
            if control.check().is_err() {
                outcomes.push(TaskOutcome {
                    id: task.id.clone(),
                    status: TaskStatus::Cancelled,
                    summary: String::new(),
                    error: Some("the caller cancelled the graph".to_owned()),
                    result: None,
                });
                break;
            }
            let blocked_by = task
                .depends_on
                .iter()
                .find(|dependency| statuses.get(*dependency) != Some(&TaskStatus::Completed));
            if let Some(dependency) = blocked_by {
                let outcome = TaskOutcome {
                    id: task.id.clone(),
                    status: TaskStatus::Blocked,
                    summary: String::new(),
                    error: Some(format!("dependency `{dependency}` did not complete")),
                    result: None,
                };
                statuses.insert(task.id.clone(), outcome.status);
                outcomes.push(outcome);
                continue;
            }
            let Some(command) = registry.get(&task.command) else {
                continue;
            };
            let args = stkcraft_astro_engine::args_from(&command.descriptor, &task.arguments);
            let enabled = registry.enabled(&task.command, scenario).unwrap_or(false);
            if !enabled {
                let outcome = TaskOutcome {
                    id: task.id.clone(),
                    status: TaskStatus::NeedsInput,
                    summary: String::new(),
                    error: Some(format!(
                        "`{}` needs objects the scenario does not have",
                        task.command
                    )),
                    result: None,
                };
                statuses.insert(task.id.clone(), outcome.status);
                outcomes.push(outcome);
                continue;
            }
            let outcome = match registry.run(&task.command, scenario, &args) {
                Ok(outcome) => TaskOutcome {
                    id: task.id.clone(),
                    status: TaskStatus::Completed,
                    summary: outcome.summary.clone(),
                    error: None,
                    result: task_result(&outcome, scenario),
                },
                Err(error) => TaskOutcome {
                    id: task.id.clone(),
                    status: TaskStatus::Failed,
                    summary: String::new(),
                    error: Some(error.to_string()),
                    result: None,
                },
            };
            statuses.insert(task.id.clone(), outcome.status);
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
}

fn task_result(outcome: &CommandOutcome, scenario: &Scenario) -> Option<Value> {
    if let Some(access) = &outcome.access {
        let pairs: Vec<Value> = access
            .iter()
            .map(|entry| {
                let satellite = scenario.satellite(entry.satellite).map_or_else(
                    || format!("{:?}", entry.satellite),
                    |satellite| satellite.name.clone(),
                );
                let station = scenario.station(entry.station).map_or_else(
                    || format!("{:?}", entry.station),
                    |station| station.name.clone(),
                );
                let windows: Vec<Value> = entry
                    .windows
                    .iter()
                    .map(|window| {
                        serde_json::json!({
                            "start_seconds": window.start.as_seconds(),
                            "end_seconds": window.end.as_seconds(),
                            "duration_seconds": window.duration().as_seconds(),
                        })
                    })
                    .collect();
                serde_json::json!({
                    "satellite": satellite,
                    "station": station,
                    "total_seconds": entry.total.as_seconds(),
                    "windows": windows,
                })
            })
            .collect();
        return Some(serde_json::json!({"kind": "access", "pairs": pairs}));
    }
    if let Some(passes) = &outcome.passes {
        let passes: Vec<Value> = passes
            .iter()
            .map(|pass| {
                serde_json::json!({
                    "start_seconds": pass.start_seconds,
                    "end_seconds": pass.end_seconds,
                    "duration_seconds": pass.duration_seconds(),
                })
            })
            .collect();
        return Some(serde_json::json!({"kind": "planet-passes", "passes": passes}));
    }
    None
}

/// A model that proposes a graph, with no tool loop.
///
/// The planner is deliberately one completion: the model receives the command catalogue and the
/// question, and returns JSON. Rust validates and executes the result; the model never dispatches a
/// command itself.
pub struct Planner<M: crate::ChatModel> {
    model: M,
}

impl<M: crate::ChatModel> Planner<M> {
    /// Build a planner over a model.
    #[must_use]
    pub fn new(model: M) -> Self {
        Self { model }
    }

    /// Ask the model for a graph.
    ///
    /// # Errors
    ///
    /// Returns a message when the model fails, when its answer is not a JSON object, or when it is
    /// not a task graph. Structural validation against the registry happens in
    /// [`TaskGraph::validate`], not here, so the caller can report both kinds of error together.
    pub fn plan(&self, question: &str, tools: &[Value]) -> Result<TaskGraph, String> {
        let system = "You are STKCraft's planner. Return ONE JSON object and nothing else, in the \
                      shape {\"tasks\":[{\"id\":\"...\",\"command\":\"...\",\"arguments\":{...},\
                      \"depends-on\":[\"...\"]}]}. Use only the commands in the catalogue. Never \
                      estimate a number. Do not add prose, markdown fences or explanations.";
        let catalogue = serde_json::to_string(tools)
            .map_err(|error| format!("could not encode the command catalogue: {error}"))?;
        let user = format!("Question: {question}\nCommand catalogue: {catalogue}");
        let turn = self
            .model
            .complete(
                &[
                    crate::Message::System(system.to_owned()),
                    crate::Message::User(user),
                ],
                &[],
            )
            .map_err(|error| format!("the planner model failed: {error}"))?;
        let json = json_object(&turn.text)
            .ok_or_else(|| "the planner did not return a JSON object".to_owned())?;
        serde_json::from_str(json).map_err(|error| format!("the plan is not a task graph: {error}"))
    }

    /// Ask for a graph, validate it, and allow exactly one correction.
    ///
    /// The second request carries the model's first answer and the validation errors. A second
    /// failure is returned; this method never loops beyond two model requests.
    ///
    /// # Errors
    ///
    /// Returns the model failure, or both validation errors when the corrected graph is still
    /// invalid.
    pub fn plan_corrected(
        &self,
        question: &str,
        tools: &[Value],
        registry: &Registry,
    ) -> Result<TaskGraph, String> {
        let system = "You are STKCraft's planner. Return ONE JSON object and nothing else, in the \
                      shape {\"tasks\":[{\"id\":\"...\",\"command\":\"...\",\"arguments\":{...},\
                      \"depends-on\":[\"...\"]}]}. Use only the commands in the catalogue. Never \
                      estimate a number. Do not add prose, markdown fences or explanations.";
        let catalogue = serde_json::to_string(tools)
            .map_err(|error| format!("could not encode the command catalogue: {error}"))?;
        let mut messages = vec![
            crate::Message::System(system.to_owned()),
            crate::Message::User(format!(
                "Question: {question}\nCommand catalogue: {catalogue}"
            )),
        ];
        let mut last_errors = Vec::new();
        for attempt in 0..2 {
            let turn = self
                .model
                .complete(&messages, &[])
                .map_err(|error| format!("the planner model failed: {error}"))?;
            let parsed = json_object(&turn.text)
                .ok_or_else(|| "the planner did not return a JSON object".to_owned())
                .and_then(|json| {
                    serde_json::from_str::<TaskGraph>(json)
                        .map_err(|error| format!("the plan is not a task graph: {error}"))
                });
            match parsed {
                Ok(graph) => match graph.validate(registry) {
                    Ok(_) => return Ok(graph),
                    Err(errors) => last_errors = errors,
                },
                Err(error) => last_errors = vec![error],
            }
            if attempt == 0 {
                messages.push(crate::Message::Assistant {
                    text: turn.text,
                    calls: Vec::new(),
                    reasoning_content: None,
                    usage: None,
                    finish_reason: None,
                });
                messages.push(crate::Message::User(format!(
                    "The plan was rejected: {}. Return one corrected JSON graph.",
                    last_errors.join("; ")
                )));
            }
        }
        Err(format!(
            "the planner did not produce a valid graph: {}",
            last_errors.join("; ")
        ))
    }
}

fn json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then_some(&text[start..=end])
}

/// A graph node's argument builder, so tests and callers do not spell JSON by hand.
#[must_use]
pub fn task(id: &str, command: &str, arguments: Value, depends_on: &[&str]) -> TaskSpec {
    TaskSpec {
        id: id.to_owned(),
        command: command.to_owned(),
        arguments,
        depends_on: depends_on.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// The registry a graph runs against.
#[must_use]
pub fn registry() -> Registry {
    Registry::builtin()
}

/// An empty argument set, for a command that takes none.
#[must_use]
pub fn no_arguments() -> Args {
    Args::none()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use serde_json::json;

    use super::*;

    const L1: &str = "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009";
    const L2: &str = "2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343";

    fn graph() -> TaskGraph {
        TaskGraph {
            tasks: vec![
                task(
                    "satellite",
                    "satellite.add",
                    json!({"name": "ISS", "line1": L1, "line2": L2}),
                    &[],
                ),
                task(
                    "station",
                    "station.add",
                    json!({
                        "name": "London",
                        "latitude": 51.5,
                        "longitude": -0.13,
                        "minimum-elevation": 10.0
                    }),
                    &[],
                ),
                task(
                    "access",
                    "access.compute",
                    json!({"step-seconds": 60}),
                    &["satellite", "station"],
                ),
            ],
        }
    }

    struct PlanModel(std::cell::RefCell<Vec<String>>);

    impl PlanModel {
        fn once(text: &str) -> Self {
            Self(std::cell::RefCell::new(vec![text.to_owned()]))
        }

        fn queue(texts: &[&str]) -> Self {
            Self(std::cell::RefCell::new(
                texts.iter().map(|text| (*text).to_owned()).collect(),
            ))
        }
    }

    impl crate::ChatModel for PlanModel {
        fn complete(
            &self,
            _messages: &[crate::Message],
            _tools: &[Value],
        ) -> Result<crate::Turn, String> {
            let mut turns = self.0.borrow_mut();
            if turns.is_empty() {
                return Err("the plan script ran out".to_owned());
            }
            Ok(crate::Turn {
                text: turns.remove(0),
                ..crate::Turn::default()
            })
        }
    }

    #[test]
    fn the_planner_extracts_a_graph_from_a_json_answer() {
        let model = PlanModel::once(
            "```json\n{\"tasks\":[{\"id\":\"rename\",\"command\":\"scenario.rename\",\
             \"arguments\":{\"name\":\"Planned\"}}]}\n```",
        );
        let graph = Planner::new(model).plan("rename", &[]).expect("a graph");
        assert_eq!(graph.tasks.len(), 1);
        assert_eq!(graph.tasks[0].id, "rename");
    }

    #[test]
    fn the_planner_corrects_one_invalid_graph() {
        let model = PlanModel::queue(&[
            "{\"tasks\":[{\"id\":\"bad\",\"command\":\"not.a.command\"}]}",
            "{\"tasks\":[{\"id\":\"rename\",\"command\":\"scenario.rename\",\
             \"arguments\":{\"name\":\"Fixed\"}}]}",
        ]);
        let graph = Planner::new(model)
            .plan_corrected("rename", &[], &registry())
            .expect("a corrected graph");
        assert_eq!(graph.tasks[0].id, "rename");
    }

    #[test]
    fn a_second_invalid_graph_is_refused() {
        let model = PlanModel::queue(&[
            "{\"tasks\":[{\"id\":\"bad\",\"command\":\"not.a.command\"}]}",
            "{\"tasks\":[{\"id\":\"bad\",\"command\":\"not.a.command\"}]}",
        ]);
        let error = Planner::new(model)
            .plan_corrected("rename", &[], &registry())
            .expect_err("still invalid");
        assert!(error.contains("did not produce a valid graph"), "{error}");
    }

    #[test]
    fn a_valid_graph_runs_in_dependency_order() {
        let mut scenario = Scenario::empty("task test");
        let outcomes = graph()
            .execute(&registry(), &mut scenario)
            .expect("a valid graph");
        assert_eq!(outcomes.len(), 3);
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.status == TaskStatus::Completed)
        );
        let access = &outcomes[2];
        assert!(access.result.is_some(), "{access:?}");
        assert_eq!(scenario.satellites.len(), 1);
        assert_eq!(scenario.stations.len(), 1);
    }

    #[test]
    fn a_failure_budget_stops_the_next_independent_task() {
        let graph = TaskGraph {
            tasks: vec![
                task(
                    "bad",
                    "satellite.add",
                    json!({"name": "bad", "line1": "bad", "line2": "bad"}),
                    &[],
                ),
                task("rename", "scenario.rename", json!({"name": "Changed"}), &[]),
            ],
        };
        let mut scenario = Scenario::empty("budget");
        let budget = TaskBudget::new(8, 1).expect("a budget");
        let outcomes = graph
            .execute_with_budget(&registry(), &mut scenario, &budget)
            .expect("valid graph");
        assert_eq!(outcomes[0].status, TaskStatus::Failed);
        assert_eq!(outcomes[1].status, TaskStatus::BudgetExceeded);
        assert_eq!(scenario.name, "budget");
    }

    #[test]
    fn a_zero_budget_is_refused() {
        assert!(TaskBudget::new(0, 1).is_err());
        assert!(TaskBudget::new(8, 0).is_err());
    }

    #[test]
    fn a_cancelled_graph_does_not_run_the_next_task() {
        let mut scenario = Scenario::empty("cancel");
        let token = crate::CancellationToken::default();
        token.cancel();
        let control = crate::RequestControl::new(token);
        let outcomes = graph()
            .execute_controlled(&registry(), &mut scenario, &control)
            .expect("valid graph");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, TaskStatus::Cancelled);
        assert_eq!(scenario.satellites.len(), 0);
    }

    #[test]
    fn a_cycle_is_refused_before_any_command_runs() {
        let graph = TaskGraph {
            tasks: vec![
                task("a", "scenario.rename", json!({"name": "A"}), &["b"]),
                task("b", "scenario.rename", json!({"name": "B"}), &["a"]),
            ],
        };
        let errors = graph.validate(&registry()).expect_err("a cycle");
        assert!(
            errors.iter().any(|error| error.contains("cycle")),
            "{errors:?}"
        );
    }

    #[test]
    fn an_unknown_command_and_a_missing_dependency_are_refused() {
        let graph = TaskGraph {
            tasks: vec![task("a", "not.a.command", json!({}), &["missing"])],
        };
        let errors = graph.validate(&registry()).expect_err("invalid");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("not in the command registry")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("not in the graph")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_failed_dependency_blocks_its_child() {
        let graph = TaskGraph {
            tasks: vec![
                task(
                    "satellite",
                    "satellite.add",
                    json!({"name": "bad", "line1": "bad", "line2": "bad"}),
                    &[],
                ),
                task(
                    "access",
                    "access.compute",
                    json!({"step-seconds": 60}),
                    &["satellite"],
                ),
            ],
        };
        let mut scenario = Scenario::empty("blocked");
        let outcomes = graph.execute(&registry(), &mut scenario).expect("valid");
        assert_eq!(outcomes[0].status, TaskStatus::Failed);
        assert_eq!(outcomes[1].status, TaskStatus::Blocked);
    }

    #[test]
    fn a_disabled_command_is_needs_input() {
        let graph = TaskGraph {
            tasks: vec![task("access", "access.compute", json!({}), &[])],
        };
        let mut scenario = Scenario::empty("empty");
        let outcomes = graph.execute(&registry(), &mut scenario).expect("valid");
        assert_eq!(outcomes[0].status, TaskStatus::NeedsInput);
    }
}
