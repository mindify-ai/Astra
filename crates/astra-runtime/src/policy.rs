//! Command policy and authorization rules.

use std::path::Path;

use astra_protocol::{Access, ApprovalPolicy, CommandPolicy, Membership, State};

/// Whether running a command needs a human approval.
///
/// `askMe` / `askSteerers` always ask. `auto` runs without asking, except commands whose working directory is
/// outside the agent's worktree, which always need approval.
pub fn command_needs_approval(policy: CommandPolicy, cwd: Option<&Path>, worktree: &Path) -> bool {
    match policy {
        CommandPolicy::AskMe | CommandPolicy::AskSteerers => true,
        CommandPolicy::Auto => match cwd {
            None => false,
            Some(c) => !within(c, worktree),
        },
    }
}

/// Lexical + canonical containment check.
pub fn within(path: &Path, root: &Path) -> bool {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    canon(path).starts_with(canon(root))
}

/// The access a person has in a mission.
pub fn access(state: &State, mission_id: &str, person_id: &str) -> Option<Access> {
    state
        .memberships
        .get(&Membership::id_for(mission_id, person_id))
        .map(|m| m.access)
}

/// Whether `actor` may decide approvals (and merge) in a mission: the owner always; members with `steer`
/// access when the mission's approval policy is `steerers` or the agent's command policy is `askSteerers`.
pub fn may_decide(
    state: &State,
    mission_id: &str,
    actor: &str,
    agent_policy: Option<CommandPolicy>,
) -> bool {
    let Some(mission) = state.missions.get(mission_id) else {
        return false;
    };
    if mission.owner_id == actor {
        return true;
    }
    let steerers_allowed = mission.approval_policy == ApprovalPolicy::Steerers
        || agent_policy == Some(CommandPolicy::AskSteerers);
    steerers_allowed
        && matches!(
            access(state, mission_id, actor),
            Some(Access::Steer | Access::Owner)
        )
}

/// Whether `actor` may steer (has owner or steer access).
/// Whether `actor` may archive or delete a mission: only its owner.
pub fn may_manage(state: &State, mission_id: &str, actor: &str) -> bool {
    state
        .missions
        .get(mission_id)
        .is_some_and(|m| m.owner_id == actor)
}

pub fn may_steer(state: &State, mission_id: &str, actor: &str) -> bool {
    state
        .missions
        .get(mission_id)
        .is_some_and(|m| m.owner_id == actor)
        || matches!(
            access(state, mission_id, actor),
            Some(Access::Steer | Access::Owner)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_rules() {
        let wt = std::env::temp_dir();
        assert!(command_needs_approval(CommandPolicy::AskMe, None, &wt));
        assert!(command_needs_approval(
            CommandPolicy::AskSteerers,
            Some(&wt),
            &wt
        ));
        assert!(!command_needs_approval(CommandPolicy::Auto, None, &wt));
        assert!(!command_needs_approval(
            CommandPolicy::Auto,
            Some(&wt.join("sub")),
            &wt
        ));
        assert!(command_needs_approval(
            CommandPolicy::Auto,
            Some(Path::new("/")),
            &wt
        ));
    }

    #[test]
    fn authorization_rules_on_seed() {
        let raw = include_str!("../../../fixtures/seed.json");
        let seed: astra_protocol::Seed = serde_json::from_str(raw).expect("seed");
        let state = State::from_seed(seed);
        assert!(may_decide(&state, "m-014", "p-mark", None));
        assert!(!may_decide(&state, "m-014", "p-lin", None)); // policy owner
        assert!(may_decide(
            &state,
            "m-014",
            "p-lin",
            Some(CommandPolicy::AskSteerers)
        ));
        assert!(!may_decide(
            &state,
            "m-014",
            "p-jo",
            Some(CommandPolicy::AskSteerers)
        )); // watch only
        assert!(may_steer(&state, "m-014", "p-lin"));
        assert!(!may_steer(&state, "m-014", "p-jo"));
    }
}
