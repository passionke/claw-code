//! Gateway capability × harness engine. The checklist is this module, not a second list.
//! Author: kejiqing
//!
//! Four cells only: honored, rejected, silent, miswired.
//! A new [`GatewayCapability`] or [`CapabilityEngine`] fails to compile until every cell is set.
//! New silent cells are forbidden by test; the current silent set is frozen in
//! [`grandfathered_silent`].

/// Harness engine the checklist is scored against. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityEngine {
    Claw,
    Opencode,
    Appserver,
}

/// Control-plane capability. Add a variant only with a cell for every engine. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayCapability {
    PlanMode,
    AskUserQuestion,
    SealedPlan,
    ProjectRoleNonNormal,
    RelaxedWorkerProfile,
    NonE2bBackend,
    PromptAndAttachments,
    ExtraSession,
    Model,
    MaxIterations,
    TimeoutSeconds,
    LandlockDsl,
    Mcp,
    InstructionsAndRules,
    Skills,
    SessionResume,
    TurnBillingHeaders,
    ThinkingEnabled,
    AllowedTools,
    SolveOrchestration,
    OtelTraceparent,
    ResponsesStream,
}

/// What the engine actually does with the capability. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityFit {
    /// Control reaches this engine and changes its behavior.
    Honored,
    /// Solve is refused (`unsupported_by_engine`) or the worker gate returns 400.
    Rejected,
    /// The switch exists on the task or project config, and this engine neither applies it nor refuses it.
    Silent,
    /// Wired to a different behavior than the control means.
    Miswired,
}

impl CapabilityEngine {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claw => "claw",
            Self::Opencode => "opencode",
            Self::Appserver => "appserver",
        }
    }

    #[must_use]
    pub fn all() -> &'static [Self] {
        &[Self::Claw, Self::Opencode, Self::Appserver]
    }
}

impl GatewayCapability {
    /// Listed in checklist order. `all_variants_are_listed` fails if a variant is missing.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::PlanMode,
            Self::AskUserQuestion,
            Self::SealedPlan,
            Self::ProjectRoleNonNormal,
            Self::RelaxedWorkerProfile,
            Self::NonE2bBackend,
            Self::PromptAndAttachments,
            Self::ExtraSession,
            Self::Model,
            Self::MaxIterations,
            Self::TimeoutSeconds,
            Self::LandlockDsl,
            Self::Mcp,
            Self::InstructionsAndRules,
            Self::Skills,
            Self::SessionResume,
            Self::TurnBillingHeaders,
            Self::ThinkingEnabled,
            Self::AllowedTools,
            Self::SolveOrchestration,
            Self::OtelTraceparent,
            Self::ResponsesStream,
        ]
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlanMode => "planMode",
            Self::AskUserQuestion => "askUserQuestion",
            Self::SealedPlan => "sealedPlan",
            Self::ProjectRoleNonNormal => "projectRoleNonNormal",
            Self::RelaxedWorkerProfile => "relaxedWorkerProfile",
            Self::NonE2bBackend => "nonE2bBackend",
            Self::PromptAndAttachments => "promptAndAttachments",
            Self::ExtraSession => "extraSession",
            Self::Model => "model",
            Self::MaxIterations => "maxIterations",
            Self::TimeoutSeconds => "timeoutSeconds",
            Self::LandlockDsl => "landlockDsl",
            Self::Mcp => "mcp",
            Self::InstructionsAndRules => "instructionsAndRules",
            Self::Skills => "skills",
            Self::SessionResume => "sessionResume",
            Self::TurnBillingHeaders => "turnBillingHeaders",
            Self::ThinkingEnabled => "thinkingEnabled",
            Self::AllowedTools => "allowedTools",
            Self::SolveOrchestration => "solveOrchestration",
            Self::OtelTraceparent => "otelTraceparent",
            Self::ResponsesStream => "responsesStream",
        }
    }

    #[must_use]
    pub fn note(self) -> &'static str {
        match self {
            Self::PlanMode => "neuro：gateway gate 与 worker reject_unsupported 均 400",
            Self::AskUserQuestion => {
                "neuro：worker 在标志仍为 true 时 400；gateway prepare_task 先写成 false，产品路径看不到 400"
            }
            Self::SealedPlan => "neuro：gateway gate 与 worker reject_unsupported 均 400",
            Self::ProjectRoleNonNormal => "neuro：validate_role 仅允许 normal",
            Self::RelaxedWorkerProfile => "neuro：validate_worker_profile 仅允许 strict",
            Self::NonE2bBackend => "neuro：gate_solve_request 拒绝非 e2b",
            Self::PromptAndAttachments => "neuro：ACP resource_link + text",
            Self::ExtraSession => "neuro：instructions 与 mcp-proxy _meta",
            Self::Model => "task.model，否则 CLAW_DEFAULT_MODEL",
            Self::MaxIterations => "neuro 按 ACP tool_call 次数取消，不是 LLM 往返次数",
            Self::TimeoutSeconds => "本轮截止；gateway 另有自己的超时",
            Self::LandlockDsl => "neuro 追加 /dev；opencode 再追加 /proc",
            Self::Mcp => "stdio/http/sse 经 mcp-proxy",
            Self::InstructionsAndRules => "CLAUDE.md 与 rules 投影到 instructions",
            Self::Skills => "opencode 丢掉不符合 ^[a-z0-9]+(-[a-z0-9]+)*$ 的名字，stderr 有记录",
            Self::SessionResume => "resume 或 loadSession；都不支持则报错，无兜底",
            Self::TurnBillingHeaders => "出站 claw-turn-id；opencode 走 header 代理，appserver 走 config.toml",
            Self::ThinkingEnabled => {
                "opencode 与 appserver 都由 localhost 代理把与 claw 相同的 chat thinking 字段写入出站 JSON"
            }
            Self::AllowedTools => "neuro 不读 task.allowedTools，MCP 按 settings 全量注入",
            Self::SolveOrchestration => {
                "只在 claw run_gateway_solve_turn 读取。neuro 不读；forceSingleTurn 被写成 true 但 neuro 也不读"
            }
            Self::OtelTraceparent => "claw worker 读取；neuro 不读",
            Self::ResponsesStream => {
                "neuro TurnMapper 用它决定是否 emit thinking.delta，不控制上游 thinking"
            }
        }
    }
}

impl CapabilityFit {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Honored => "落实",
            Self::Rejected => "拒绝",
            Self::Silent => "静默",
            Self::Miswired => "错接",
        }
    }
}

/// Cell for one engine. Exhaustive: a new capability or engine does not compile here.
/// Author: kejiqing
#[must_use]
pub fn capability_fit(engine: CapabilityEngine, capability: GatewayCapability) -> CapabilityFit {
    use CapabilityEngine::{Appserver, Claw, Opencode};
    use CapabilityFit::{Honored, Miswired, Rejected, Silent};
    use GatewayCapability::{
        AllowedTools, AskUserQuestion, ExtraSession, InstructionsAndRules, LandlockDsl,
        MaxIterations, Mcp, Model, NonE2bBackend, OtelTraceparent, PlanMode, ProjectRoleNonNormal,
        PromptAndAttachments, RelaxedWorkerProfile, ResponsesStream, SealedPlan, SessionResume,
        Skills, SolveOrchestration, ThinkingEnabled, TimeoutSeconds, TurnBillingHeaders,
    };
    match (engine, capability) {
        (
            Claw,
            PlanMode | AskUserQuestion | SealedPlan | ProjectRoleNonNormal | RelaxedWorkerProfile
            | NonE2bBackend | PromptAndAttachments | ExtraSession | Model | MaxIterations
            | TimeoutSeconds | LandlockDsl | Mcp | InstructionsAndRules | Skills | SessionResume
            | TurnBillingHeaders | ThinkingEnabled | AllowedTools | SolveOrchestration
            | OtelTraceparent | ResponsesStream,
        ) => Honored,
        (
            Opencode | Appserver,
            PlanMode | AskUserQuestion | SealedPlan | ProjectRoleNonNormal | RelaxedWorkerProfile
            | NonE2bBackend,
        ) => Rejected,
        (
            Opencode | Appserver,
            PromptAndAttachments | ExtraSession | Model | MaxIterations | TimeoutSeconds
            | LandlockDsl | Mcp | InstructionsAndRules | Skills | SessionResume
            | TurnBillingHeaders | ThinkingEnabled,
        ) => Honored,
        (Opencode | Appserver, AllowedTools | SolveOrchestration | OtelTraceparent) => Silent,
        (Opencode | Appserver, ResponsesStream) => Miswired,
    }
}

/// Silent cells that already exist. A new silent cell fails `silent_cells_stay_inside_the_frozen_set`.
/// Closing one of these requires editing this function in the same change as [`capability_fit`].
/// Author: kejiqing
#[must_use]
pub fn grandfathered_silent(engine: CapabilityEngine, capability: GatewayCapability) -> bool {
    use CapabilityEngine::{Appserver, Opencode};
    use GatewayCapability::{AllowedTools, OtelTraceparent, SolveOrchestration};
    matches!(
        (engine, capability),
        (
            Opencode | Appserver,
            AllowedTools | SolveOrchestration | OtelTraceparent
        )
    )
}

/// Human checklist rendered from the enum. Tests lock this text. Author: kejiqing
#[must_use]
pub fn render_capability_matrix() -> String {
    let mut out = String::from(
        "ACP 能力总图\n真源：gateway_solve_turn::capability_matrix。四态：落实、拒绝、静默、错接。\n\n",
    );
    out.push_str("| 能力 | claw | opencode | appserver | 说明 |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for capability in GatewayCapability::all() {
        let cells: Vec<&str> = CapabilityEngine::all()
            .iter()
            .map(|engine| capability_fit(*engine, *capability).as_str())
            .collect();
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            capability.as_str(),
            cells[0],
            cells[1],
            cells[2],
            capability.note()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_are_listed() {
        fn index(capability: GatewayCapability) -> usize {
            match capability {
                GatewayCapability::PlanMode => 0,
                GatewayCapability::AskUserQuestion => 1,
                GatewayCapability::SealedPlan => 2,
                GatewayCapability::ProjectRoleNonNormal => 3,
                GatewayCapability::RelaxedWorkerProfile => 4,
                GatewayCapability::NonE2bBackend => 5,
                GatewayCapability::PromptAndAttachments => 6,
                GatewayCapability::ExtraSession => 7,
                GatewayCapability::Model => 8,
                GatewayCapability::MaxIterations => 9,
                GatewayCapability::TimeoutSeconds => 10,
                GatewayCapability::LandlockDsl => 11,
                GatewayCapability::Mcp => 12,
                GatewayCapability::InstructionsAndRules => 13,
                GatewayCapability::Skills => 14,
                GatewayCapability::SessionResume => 15,
                GatewayCapability::TurnBillingHeaders => 16,
                GatewayCapability::ThinkingEnabled => 17,
                GatewayCapability::AllowedTools => 18,
                GatewayCapability::SolveOrchestration => 19,
                GatewayCapability::OtelTraceparent => 20,
                GatewayCapability::ResponsesStream => 21,
            }
        }
        let mut seen = [false; 22];
        for capability in GatewayCapability::all() {
            seen[index(*capability)] = true;
        }
        assert!(seen.iter().all(|hit| *hit));
    }

    #[test]
    fn silent_cells_stay_inside_the_frozen_set() {
        for engine in CapabilityEngine::all() {
            for capability in GatewayCapability::all() {
                let silent = capability_fit(*engine, *capability) == CapabilityFit::Silent;
                let frozen = grandfathered_silent(*engine, *capability);
                assert_eq!(
                    silent,
                    frozen,
                    "{} {} silent={silent} frozen={frozen}",
                    engine.as_str(),
                    capability.as_str()
                );
            }
        }
    }

    #[test]
    fn capability_matrix_snapshot_is_the_checklist() {
        let rendered = render_capability_matrix();
        assert_eq!(rendered, include_str!("capability_matrix_snapshot.txt"));
    }
}
