//! Permission catalog: the resource/action registry and the built-in roles.
//!
//! This mirrors the reference's `service/authz` package
//! (`registry.go`, `role.go`, `resources_*.go`) and serves the same contract:
//! `GET /api/authz/catalog` returns every resource with its actions, plus each
//! role's baseline grant matrix. It is the schema a permission editor renders and
//! the statement of what an admin may do by default.
//!
//! Two properties are deliberate, both mirroring the reference:
//!
//! * **The registry is the single source of truth.** Actions carry the `DefaultRoles`
//!   that receive them, and grants are *computed* from that rather than stored
//!   separately. A stored matrix could contradict the registry; a computed one
//!   cannot.
//! * **`DefaultRoles` is not serialized.** The reference tags it `json:"-"`, so the
//!   client learns grants through `roles[].grants` rather than by reading which
//!   role each action names. Matching that keeps the two shapes interchangeable.
//!
//! Scope note: this is the catalog, not enforcement. The reference additionally
//! evaluates these permissions through Casbin on every admin route; we do not yet,
//! and `docs/FACT.md` records that gap rather than implying parity.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde::{Deserialize, Serialize};

/// The root role. Granted every permission without an explicit entry.
pub const BUILT_IN_ROLE_ROOT: &str = "root";

/// The admin role. Granted exactly the actions whose `default_roles` name it.
pub const BUILT_IN_ROLE_ADMIN: &str = "admin";

/// One action a resource exposes.
#[derive(Debug, Clone)]
pub struct ActionDefinition {
    pub action: &'static str,
    /// Display text for the action.
    pub label: &'static str,
    /// Longer explanation, shown as help in a permission editor.
    pub description: &'static str,
    /// Roles that receive this action in their baseline grant.
    ///
    /// Never serialized: a client reads grants from [`RoleDescriptor::grants`].
    pub default_roles: &'static [&'static str],
}

/// One resource and the actions it exposes.
#[derive(Debug, Clone)]
pub struct ResourceDefinition {
    pub resource: &'static str,
    pub label: &'static str,
    pub actions: &'static [ActionDefinition],
}

impl ResourceDefinition {
    /// Whether this resource exposes `action`.
    pub fn has_action(&self, action: &str) -> bool {
        self.actions.iter().any(|a| a.action == action)
    }
}

// ── The registry ──────────────────────────────────────────────────────────

/// Actions shared across resources.
pub const ACTION_READ: &str = "read";
pub const ACTION_OPERATE: &str = "operate";
pub const ACTION_WRITE: &str = "write";
pub const ACTION_SENSITIVE_WRITE: &str = "sensitive_write";
pub const ACTION_SECRET_VIEW: &str = "secret_view";
pub const ACTION_BIND: &str = "bind";

/// Channel management, split by how dangerous each action is.
///
/// The split is the point: reading a channel list, testing a channel, editing its
/// routing, rewriting its credential, and reading its credential back are five
/// different privileges. Collapsing them into one "admin" bit is what this catalog
/// exists to avoid.
const CHANNEL_ACTIONS: &[ActionDefinition] = &[
    ActionDefinition {
        action: ACTION_READ,
        label: "Read channels",
        description: "View channel lists and details without secrets.",
        default_roles: &[BUILT_IN_ROLE_ADMIN],
    },
    ActionDefinition {
        action: ACTION_OPERATE,
        label: "Operate channels",
        description: "Test channels, refresh balances, and enable or disable individual, batch, or tagged channels.",
        default_roles: &[BUILT_IN_ROLE_ADMIN],
    },
    ActionDefinition {
        action: ACTION_WRITE,
        label: "Edit channel routing",
        description: "Edit non-sensitive settings such as models, groups, and routing rules.",
        default_roles: &[BUILT_IN_ROLE_ADMIN],
    },
    ActionDefinition {
        action: ACTION_SENSITIVE_WRITE,
        label: "Edit sensitive channel settings",
        description: "Create channels or edit keys, base URLs, and overrides.",
        default_roles: &[],
    },
    ActionDefinition {
        action: ACTION_SECRET_VIEW,
        label: "View channel secrets",
        description: "Reserved for viewing complete channel keys after secure verification.",
        default_roles: &[],
    },
];

const AUDIT_ACTIONS: &[ActionDefinition] = &[ActionDefinition {
    action: ACTION_READ,
    label: "View other accounts' audit logs",
    description: "View audit records from user and admin roles. Root records are always excluded.",
    default_roles: &[],
}];

const TASK_PLUGIN_ACTIONS: &[ActionDefinition] = &[ActionDefinition {
    action: ACTION_BIND,
    label: "Bind task plugins",
    description: "List registered task plugins and bind them when creating or editing task plugin channels.",
    default_roles: &[],
}];

/// The complete registry. Order defines presentation order.
pub const RESOURCE_REGISTRY: &[ResourceDefinition] = &[
    ResourceDefinition {
        resource: "channel",
        label: "Channel Management",
        actions: CHANNEL_ACTIONS,
    },
    ResourceDefinition {
        resource: "audit",
        label: "Audit Logs",
        actions: AUDIT_ACTIONS,
    },
    ResourceDefinition {
        resource: "task_plugin",
        label: "Task Plugin",
        actions: TASK_PLUGIN_ACTIONS,
    },
];

/// Whether `resource` is registered.
pub fn is_known_resource(resource: &str) -> bool {
    RESOURCE_REGISTRY.iter().any(|r| r.resource == resource)
}

/// Whether `resource`/`action` is a registered permission.
pub fn is_known_permission(resource: &str, action: &str) -> bool {
    RESOURCE_REGISTRY
        .iter()
        .find(|r| r.resource == resource)
        .map(|r| r.has_action(action))
        .unwrap_or(false)
}

/// Every registered permission, as `(resource, action)`.
pub fn all_permissions() -> Vec<(&'static str, &'static str)> {
    RESOURCE_REGISTRY
        .iter()
        .flat_map(|r| r.actions.iter().map(move |a| (r.resource, a.action)))
        .collect()
}

/// The permissions whose baseline grant includes `role`.
pub fn permissions_for_role(role: &str) -> Vec<(&'static str, &'static str)> {
    RESOURCE_REGISTRY
        .iter()
        .flat_map(|r| {
            r.actions
                .iter()
                .filter(|a| a.default_roles.contains(&role))
                .map(move |a| (r.resource, a.action))
        })
        .collect()
}

/// Whether `role` receives `action` on `resource` in its baseline grant.
pub fn role_has_permission(role: &str, resource: &str, action: &str) -> bool {
    // The root role is a superuser: every permission, with no explicit entry.
    if role == BUILT_IN_ROLE_ROOT {
        return is_known_permission(resource, action);
    }
    RESOURCE_REGISTRY
        .iter()
        .find(|r| r.resource == resource)
        .and_then(|r| r.actions.iter().find(|a| a.action == action))
        .map(|a| a.default_roles.contains(&role))
        .unwrap_or(false)
}

// ── Serialized catalog ────────────────────────────────────────────────────

/// One action as the client sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogAction {
    pub action: String,
    pub label_key: String,
    pub description_key: String,
}

/// One resource as the client sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogResource {
    pub resource: String,
    pub label_key: String,
    pub actions: Vec<CatalogAction>,
}

/// A role with its baseline grant matrix.
///
/// `grants` is `resource -> action -> allowed`, with an entry for every registered
/// action so a client can render a complete grid without inferring defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleDescriptor {
    pub key: String,
    pub name: String,
    pub built_in: bool,
    pub superuser: bool,
    pub grants: std::collections::BTreeMap<String, std::collections::BTreeMap<String, bool>>,
}

/// The catalog payload: every resource, and every role's baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionCatalog {
    pub resources: Vec<CatalogResource>,
    pub roles: Vec<RoleDescriptor>,
}

/// Build the catalog.
pub fn catalog() -> PermissionCatalog {
    let resources: Vec<CatalogResource> = RESOURCE_REGISTRY
        .iter()
        .map(|r| CatalogResource {
            resource: r.resource.to_string(),
            label_key: r.label.to_string(),
            actions: r
                .actions
                .iter()
                .map(|a| CatalogAction {
                    action: a.action.to_string(),
                    label_key: a.label.to_string(),
                    description_key: a.description.to_string(),
                })
                .collect(),
        })
        .collect();

    let roles = vec![
        role_descriptor(BUILT_IN_ROLE_ROOT, "Root", true),
        role_descriptor(BUILT_IN_ROLE_ADMIN, "Admin", false),
    ];

    PermissionCatalog { resources, roles }
}

fn role_descriptor(key: &str, name: &str, superuser: bool) -> RoleDescriptor {
    let mut grants = std::collections::BTreeMap::new();
    for resource in RESOURCE_REGISTRY {
        let actions = resource
            .actions
            .iter()
            .map(|a| {
                // Computed from the registry, never stored, so the matrix cannot
                // disagree with the action definitions.
                let allowed = superuser || a.default_roles.contains(&key);
                (a.action.to_string(), allowed)
            })
            .collect();
        grants.insert(resource.resource.to_string(), actions);
    }
    RoleDescriptor {
        key: key.to_string(),
        name: name.to_string(),
        built_in: true,
        superuser,
        grants,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_holds_the_three_reference_resources() {
        let names: Vec<&str> = RESOURCE_REGISTRY.iter().map(|r| r.resource).collect();
        assert_eq!(names, vec!["channel", "audit", "task_plugin"]);
    }

    #[test]
    fn channel_exposes_five_distinct_privileges() {
        // The split is the reason the catalog exists: reading a channel list and
        // rewriting its credential must not be the same privilege.
        let channel = RESOURCE_REGISTRY
            .iter()
            .find(|r| r.resource == "channel")
            .unwrap();
        let actions: Vec<&str> = channel.actions.iter().map(|a| a.action).collect();
        assert_eq!(
            actions,
            vec!["read", "operate", "write", "sensitive_write", "secret_view"]
        );
    }

    #[test]
    fn the_admin_baseline_covers_routing_but_not_secrets() {
        // An admin may read, operate and retune channels; creating one or reading
        // its key is deliberately not in the baseline.
        assert!(role_has_permission(BUILT_IN_ROLE_ADMIN, "channel", "read"));
        assert!(role_has_permission(BUILT_IN_ROLE_ADMIN, "channel", "operate"));
        assert!(role_has_permission(BUILT_IN_ROLE_ADMIN, "channel", "write"));
        assert!(!role_has_permission(
            BUILT_IN_ROLE_ADMIN,
            "channel",
            "sensitive_write"
        ));
        assert!(!role_has_permission(
            BUILT_IN_ROLE_ADMIN,
            "channel",
            "secret_view"
        ));
    }

    #[test]
    fn the_admin_baseline_excludes_audit_and_plugin_binding() {
        assert!(!role_has_permission(BUILT_IN_ROLE_ADMIN, "audit", "read"));
        assert!(!role_has_permission(
            BUILT_IN_ROLE_ADMIN,
            "task_plugin",
            "bind"
        ));
    }

    #[test]
    fn root_holds_every_registered_permission() {
        for (resource, action) in all_permissions() {
            assert!(
                role_has_permission(BUILT_IN_ROLE_ROOT, resource, action),
                "root should hold {resource}/{action}"
            );
        }
    }

    #[test]
    fn an_unknown_permission_is_not_granted_to_anyone() {
        // A superuser is not a wildcard: an action that does not exist must not
        // read as allowed, or a typo in a check would silently pass.
        assert!(!role_has_permission(
            BUILT_IN_ROLE_ROOT,
            "channel",
            "not_an_action"
        ));
        assert!(!role_has_permission(
            BUILT_IN_ROLE_ROOT,
            "not_a_resource",
            "read"
        ));
        assert!(!role_has_permission(
            BUILT_IN_ROLE_ADMIN,
            "channel",
            "not_an_action"
        ));
    }

    #[test]
    fn known_permission_checking_is_exact() {
        assert!(is_known_resource("channel"));
        assert!(!is_known_resource("Channel"));
        assert!(is_known_permission("channel", "read"));
        assert!(!is_known_permission("channel", "read_all"));
        // An action registered under one resource is not valid under another.
        assert!(!is_known_permission("audit", "write"));
    }

    #[test]
    fn the_catalog_serializes_with_the_reference_field_names() {
        let catalog = catalog();
        let value = serde_json::to_value(&catalog).unwrap();

        assert!(value["resources"].is_array());
        let first = &value["resources"][0];
        assert_eq!(first["resource"], "channel");
        assert_eq!(first["label_key"], "Channel Management");
        let action = &first["actions"][0];
        assert_eq!(action["action"], "read");
        assert_eq!(action["label_key"], "Read channels");
        assert!(action["description_key"].is_string());

        // `default_roles` must not leak into the wire shape: the reference tags
        // it `json:"-"`, and the client learns grants from `roles[].grants`.
        assert!(
            action.get("default_roles").is_none(),
            "default_roles must not be serialized"
        );

        let root = &value["roles"][0];
        assert_eq!(root["key"], "root");
        assert_eq!(root["built_in"], true);
        assert_eq!(root["superuser"], true);
        let admin = &value["roles"][1];
        assert_eq!(admin["key"], "admin");
        assert_eq!(admin["superuser"], false);
    }

    #[test]
    fn every_role_grant_matrix_covers_every_registered_action() {
        // A client renders a grid, so a missing cell would be a rendering hole
        // rather than a visible error.
        let catalog = catalog();
        for role in &catalog.roles {
            for resource in RESOURCE_REGISTRY {
                let actions = role
                    .grants
                    .get(resource.resource)
                    .unwrap_or_else(|| panic!("{} missing {}", role.key, resource.resource));
                for action in resource.actions {
                    assert!(
                        actions.contains_key(action.action),
                        "{} missing {}/{}",
                        role.key,
                        resource.resource,
                        action.action
                    );
                }
            }
        }
    }

    #[test]
    fn computed_grants_agree_with_the_registry_for_every_cell() {
        // The invariant that makes a computed matrix better than a stored one.
        let catalog = catalog();
        for role in &catalog.roles {
            for resource in RESOURCE_REGISTRY {
                for action in resource.actions {
                    let computed = role.grants[resource.resource][action.action];
                    let registered = role_has_permission(role.key.as_str(), resource.resource, action.action);
                    assert_eq!(
                        computed, registered,
                        "{}/{} mismatch for role {}",
                        resource.resource, action.action, role.key
                    );
                }
            }
        }
    }

    #[test]
    fn permissions_for_role_lists_exactly_the_admin_baseline() {
        let admin = permissions_for_role(BUILT_IN_ROLE_ADMIN);
        // The three routing-safe channel actions, and nothing else.
        assert_eq!(admin.len(), 3);
        assert!(admin.contains(&("channel", "read")));
        assert!(admin.contains(&("channel", "operate")));
        assert!(admin.contains(&("channel", "write")));
        // Root is a superuser, so it has no explicit baseline entries.
        assert!(permissions_for_role(BUILT_IN_ROLE_ROOT).is_empty());
    }
}
