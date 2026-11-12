Warning: truncated output (original token count: 29800)
Total output lines: 3200

//! Retained-mode component tree for application interfaces.
//!
//! The tree owns component identity, hierarchy, layout participation, and
//! interaction dispatch. Components render directly into GhostEngine's CPU framebuffer.

use std::collections::HashMap;

use crate::rasterizer::fill_rect;
use crate::{
    draw_button, draw_checkbox, draw_combo_box, draw_label_clipped, draw_panel, draw_radio_button,
    draw_slider, draw_switch, draw_tabs, draw_text_input, ComboBoxState, FontFace, Framebuffer,
    InteractionEvents, LayoutItem, ListOptions, ListState, Point, Rect, StackLayout, TabsState,
    TextInputOptions, TextInputState, UiInteraction, UiTheme, WidgetState,
};

/// Stable caller-assigned identity for a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(pub u64);

/// The semantic role and user-owned data of a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentKind {
    Panel,
    Button {
        label: String,
    },
    Checkbox {
        label: String,
        checked: bool,
    },
    TextInput {
        placeholder: String,
        value: String,
    },
    Label {
        text: String,
    },
    ComboBox {
        items: Vec<String>,
    },
    Tabs {
        labels: Vec<String>,
    },
    Switch {
        label: String,
        enabled: bool,
    },
    RadioButton {
        label: String,
        group: u64,
        selected: bool,
    },
    Slider {
        label: String,
        min: u32,
        max: u32,
        value: u32,
    },
    ListView {
        items: Vec<String>,
        row_height: u32,
        scroll_offset: u32,
    },
}

/// A retained component node. Child order is also layout and keyboard tab order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiComponent {
    id: ComponentId,
    kind: ComponentKind,
    enabled: bool,
    visible: bool,
    layout_item: LayoutItem,
    child_layout: StackLayout,
    tooltip: Option<String>,
    children: Vec<Self>,
}

impl UiComponent {
    #[must_use]
    pub fn panel(id: ComponentId, layout: StackLayout) -> Self {
        Self::new(id, ComponentKind::Panel).with_child_layout(layout)
    }

    #[must_use]
    pub fn button(id: ComponentId, label: impl Into<String>) -> Self {
        Self::new(
            id,
            ComponentKind::Button {
                label: label.into(),
            },
        )
    }

    #[must_use]
    pub fn checkbox(id: ComponentId, label: impl Into<String>, checked: bool) -> Self {
        Self::new(
            id,
            ComponentKind::Checkbox {
                label: label.into(),
                checked,
            },
        )
    }

    #[must_use]
    pub fn text_input(
        id: ComponentId,
        placeholder: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self::new(
            id,
            ComponentKind::TextInput {
                placeholder: placeholder.into(),
                value: value.into(),
            },
        )
    }

    #[must_use]
    pub fn label(id: ComponentId, text: impl Into<String>) -> Self {
        Self::new(id, ComponentKind::Label { text: text.into() })
    }

    /// Creates a keyboard- and pointer-operable switch.
    #[must_use]
    pub fn switch(id: ComponentId, label: impl Into<String>, enabled: bool) -> Self {
        Self::new(
            id,
            ComponentKind::Switch {
                label: label.into(),
                enabled,
            },
        )
    }

    /// Creates a radio button belonging to an exclusive selection group.
    #[must_use]
    pub fn radio_button(
        id: ComponentId,
        label: impl Into<String>,
        group: u64,
        selected: bool,
    ) -> Self {
        Self::new(
            id,
            ComponentKind::RadioButton {
                label: label.into(),
                group,
                selected,
            },
        )
    }

    /// Creates an integer slider with a clamped initial value.
    #[must_use]
    pub fn slider(
        id: ComponentId,
        label: impl Into<String>,
        min: u32,
        max: u32,
        value: u32,
    ) -> Self {
        let low = min.min(max);
        let high = max.max(min);
        Self::new(
            id,
            ComponentKind::Slider {
                label: label.into(),
                min: low,
                max: high,
                value: value.clamp(low, high),
            },
        )
    }

    /// Creates a keyboard- and pointer-operable combo box.
    #[must_use]
    pub fn combo_box(id: ComponentId, items: Vec<String>) -> Self {
        Self::new(id, ComponentKind::ComboBox { items })
    }

    /// Creates a retained list view with a fixed row height.
    #[must_use]
    pub fn list_view(id: ComponentId, items: Vec<String>, row_height: u32) -> Self {
        Self::new(
            id,
            ComponentKind::ListView {
                items,
                row_height: row_height.max(1),
                scroll_offset: 0,
            },
        )
    }

    /// Creates a keyboard- and pointer-operable horizontal tab strip.
    #[must_use]
    pub fn tabs(id: ComponentId, labels: Vec<String>) -> Self {
        Self::new(id, ComponentKind::Tabs { labels })
    }

    #[must_use]
    pub fn new(id: ComponentId, kind: ComponentKind) -> Self {
        Self {
            id,
            kind,
            enabled: true,
            visible: true,
            layout_item: LayoutItem::flexible(),
            child_layout: StackLayout::default(),
            tooltip: None,
            children: Vec::new(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> ComponentId {
        self.id
    }

    #[must_use]
    pub fn kind(&self) -> &ComponentKind {
        &self.kind
    }

    #[must_use]
    pub fn children(&self) -> &[Self] {
        &self.children
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    #[must_use]
    pub fn with_layout_item(mut self, item: LayoutItem) -> Self {
        self.layout_item = item;
        self
    }

    #[must_use]
    pub fn with_child_layout(mut self, layout: StackLayout) -> Self {
        self.child_layout = layout;
        self
    }

    /// Adds a tooltip displayed while this component is hovered.
    #[must_use]
    pub fn with_tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    #[must_use]
    pub fn with_children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn set_checked(&mut self, checked: bool) -> bool {
        if let ComponentKind::Checkbox { checked: value, .. } = &mut self.kind {
            *value = checked;
            true
        } else {
            false
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        if self.enabled == enabled {
            return false;
        }
        self.enabled = enabled;
        true
    }

    pub fn set_visible(&mut self, visible: bool) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        true
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> bool {
        let text = text.into();
        match &mut self.kind {
            ComponentKind::TextInput { value, .. } => *value = text,
            ComponentKind::Label { text: value } => *value = text,
            _ => return false,
        }
        true
    }

    fn is_focusable(&self) -> bool {
        self.enabled
            && self.visible
            && matches!(
                self.kind,
                ComponentKind::Button { .. }
                    | ComponentKind::Checkbox { .. }
                    | ComponentKind::TextInput { .. }
                    | ComponentKind::ComboBox { .. }
                    | ComponentKind::Tabs { .. }
                    | ComponentKind::Switch { .. }
                    | ComponentKind::RadioButton { .. }
                    | ComponentKind::Slider { .. }
                    | ComponentKind::ListView { .. }
            )
    }
}

/// Discrete keys routed by the retained component tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKey {
    Tab,
    Enter,
    Space,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Escape,
    WordLeft,
    WordRight,
    Home,
    End,
    Backspace,
    Delete,
    DeleteWordBackward,
    DeleteWordForward,
}

/// A component's computed geometry and visual interaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedComponent {
    pub id: ComponentId,
    pub bounds: Rect,
    pub state: WidgetState,
}

/// Keyboard modifiers required to activate a context-menu command shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextMenuShortcut {
    pub key: char,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

impl ContextMenuShortcut {
    #[must_use]
    pub fn new(key: char) -> Self {
        Self {
            key: key.to_ascii_lowercase(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_control(mut self, enabled: bool) -> Self {
        self.control = enabled;
        self
    }

    #[must_use]
    pub fn with_alt(mut self, enabled: bool) -> Self {
        self.alt = enabled;
        self
    }

    #[must_use]
    pub fn with_shift(mut self, enabled: bool) -> Self {
        self.shift = enabled;
        self
    }
}

/// A selectable command displayed by a retained context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuItem {
    pub action: u64,
    pub label: String,
    pub enabled: bool,
    /// Optional configurable keyboard shortcut for this command.
    pub shortcut: Option<ContextMenuShortcut>,
    /// Child commands displayed in a submenu instead of activating this item.
    pub submenu: Vec<ContextMenuItem>,
}

impl ContextMenuItem {
    #[must_use]
    pub fn new(action: u64, label: impl Into<String>) -> Self {
        Self {
            action,
            label: label.into(),
            enabled: true,
            shortcut: None,
            submenu: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Assigns a keyboard shortcut to this command.
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: ContextMenuShortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Adds commands opened as a submenu when this item is selected.
    #[must_use]
    pub fn with_submenu(mut self, items: Vec<ContextMenuItem>) -> Self {
        self.submenu = items;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContextMenuState {
    position: Point,
    selected: usize,
    submenu_selected: usize,
    submenu_open: bool,
    submenu_path: [usize; 8],
    submenu_depth: usize,
    target: Option<ComponentId>,
}

/// Component-level events emitted by pointer or keyboard interaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentEvents {
    pub activated: Option<ComponentId>,
    pub focus_changed: Option<ComponentId>,
    /// Host clipboard operation requested by a keyboard shortcut.
    pub clipboard_action: Option<ClipboardAction>,
    /// Application-defined action selected from a context menu.
    pub context_menu_action: Option<u64>,
    /// Component on which the context menu was opened, when applicable.
    pub context_menu_target: Option<ComponentId>,
}

/// Clipboard operations are delegated to the host application/window system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardAction {
    Copy(String),
    Paste,
}

/// Text delivered by a platform text service, independent of physical keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputEvent {
    Insert(String),
    CompositionUpdate(String),
    CompositionCommit(String),
    CompositionCancel,
}

/// Errors returned by checked component-tree lifecycle operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentTreeError {
    DuplicateId(ComponentId),
    ParentNotFound(ComponentId),
}

impl std::fmt::Display for ComponentTreeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(formatter, "duplicate component ID {}", id.0),
            Self::ParentNotFound(id) => {
                write!(formatter, "parent component {} was not found", id.0)
            }
        }
    }
}

impl std::error::Error for ComponentTreeError {}

fn format_context_menu_item_label(item: &ContextMenuItem) -> String {
    let mut label = item.label.clone();
    if !item.submenu.is_empty() {
        label.push_str("  >");
    }
    if let Some(shortcut) = item.shortcut {
        label.push_str("    ");
        if shortcut.control {
            label.push_str("Ctrl+");
        }
        if shortcut.alt {
            label.push_str("Alt+");
        }
        if shortcut.shift {
            label.push_str("Shift+");
        }
        label.push(shortcut.key.to_ascii_uppercase());
    }
    label
}

/// Retained component hierarchy with layout and interaction dispatch.
#[derive(Debug, Clone, Default)]
pub struct ComponentTree {
    roots: Vec<UiComponent>,
    interaction: UiInteraction,
    focusable_ids: Vec<ComponentId>,
    focusable_bounds: Vec<Rect>,
    placed: Vec<PlacedComponent>,
    text_states: HashMap<ComponentId, TextInputState>,
    combo_states: HashMap<ComponentId, ComboBoxState>,
    tabs_states: HashMap<ComponentId, TabsState>,
    list_states: HashMap<ComponentId, ListState>,
    context_menu_items: Vec<ContextMenuItem>,
    active_context_menu_items: Vec<ContextMenuItem>,
    component_context_menus: HashMap<ComponentId, Vec<ContextMenuItem>>,
    context_menu_state: Option<ContextMenuState>,
}

impl ComponentTree {
    #[must_use]
    pub fn new(roots: Vec<UiComponent>) -> Self {
        let mut text_states = HashMap::new();
        let mut combo_states = HashMap::new();
        let mut tabs_states = HashMap::new();
        let mut list_states = HashMap::new();
        for root in &roots {
            collect_component_states(
                root,
                &mut text_states,
                &mut combo_states,
                &mut tabs_states,
                &mut list_states,
            );
        }
        Self {
            roots,
            interaction: UiInteraction::new(),
            focusable_ids: Vec::new(),
            focusable_bounds: Vec::new(),
            placed: Vec::new(),
            text_states,
            combo_states,
            tabs_states,
            list_states,
            context_menu_items: Vec::new(),
            active_context_menu_items: Vec::new(),
            component_context_menus: HashMap::new(),
            context_menu_state: None,
        }
    }

    /// Builds a tree only when every component ID is unique.
    pub fn try_new(roots: Vec<UiComponent>) -> Result<Self, ComponentTreeError> {
        validate_unique_ids(&roots)?;
        Ok(Self::new(roots))
    }

    /// Adds a new root component. IDs must be unique across the whole tree.
    pub fn add_root(&mut self, component: UiComponent) -> Result<(), ComponentTreeError> {
        validate_subtree_ids(&component)?;
        for id in component_ids(&component) {
            if self.find(id).is_some() {
                return Err(ComponentTreeError::DuplicateId(id));
            }
        }
        collect_component_states(
            &component,
            &mut self.text_states,
            &mut self.combo_states,
            &mut self.tabs_states,
            &mut self.list_states,
        );
        self.roots.push(component);
        self.reset_after_structure_change();
        Ok(())
    }

    /// Adds a child to an existing parent, preserving the supplied child order.
    pub fn add_child(
        &mut self,
        parent: ComponentId,
        child: UiComponent,
    ) -> Result<(), ComponentTreeError> {
        validate_subtree_ids(&child)?;
        for id in component_ids(&child) {
            if self.find(id).is_some() {
                return Err(ComponentTreeError::DuplicateId(id));
            }
        }
        if self.find(parent).is_none() {
            return Err(ComponentTreeError::ParentNotFound(parent));
        }
        collect_component_states(
            &child,
            &mut self.text_states,
            &mut self.combo_states,
            &mut self.tabs_states,
            &mut self.list_states,
        );
        self.find_mut(parent)
            .expect("parent was checked above")
            .children
            .push(child);
        self.reset_after_structure_change();
        Ok(())
    }

    /// Changes whether a component can receive pointer or keyboard input.
    /// Returns false when the ID is missing or the value is unchanged.
    pub fn set_enabled(&mut self, id: ComponentId, enabled: bool) -> bool {
        let changed = self
            .find_mut(id)
            .is_some_and(|component| component.set_enabled(enabled));
        if changed {
            self.reset_after_structure_change();
        }
        changed
    }

    /// Changes component visibility. Hidden subtrees are omitted from layout and input.
    /// Returns false when the ID is missing or the value is unchanged.
    pub fn set_visible(&mut self, id: ComponentId, visible: bool) -> bool {
        let changed = self
            .find_mut(id)
            .is_some_and(|component| component.set_visible(visible));
        if changed {
            self.reset_after_structure_change();
        }
        changed
    }

    /// Updates a checkbox value by stable ID.
    pub fn set_checked(&mut self, id: ComponentId, checked: bool) -> bool {
        self.find_mut(id)
            .is_some_and(|component| component.set_checked(checked))
    }

    /// Updates a slider value, clamping it to the configured range.
    pub fn set_slider_value(&mut self, id: ComponentId, value: u32) -> bool {
        let Some(ComponentKind::Slider {
            min,
            max,
            value: current,
            ..
        }) = self.find_mut(id).map(|component| &mut component.kind)
        else {
            return false;
        };
        *current = value.clamp(*min, *max);
        true
    }

    #[must_use]
    pub fn slider_value(&self, id: ComponentId) -> Option<u32> {
        match self.find(id)?.kind() {
            ComponentKind::Slider { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Updates a label or text input by stable ID.
    pub fn set_text(&mut self, id: ComponentId, text: impl Into<String>) -> bool {
        let text = text.into();
        let changed = self
            .find_mut(id)
            .is_some_and(|component| component.set_text(text.clone()));
        if changed {
            if let Some(state) = self.text_states.get_mut(&id) {
                state.set_text(text);
            }
        }
        changed
    }

    /// Removes a component and its entire descendant subtree.
    /// Returns the removed node, or None when the ID does not exist.
    pub fn remove(&mut self, id: ComponentId) -> Option<UiComponent> {
        let removed = remove_component(&mut self.roots, id)?;
        for removed_id in component_ids(&removed) {
            self.text_states.remove(&removed_id);
            self.combo_states.remove(&removed_id);
            self.tabs_states.remove(&removed_id);
            self.list_states.remove(&removed_id);
            self.component_context_menus.remove(&removed_id);
            if self
                .context_menu_state
                .is_some_and(|menu| menu.target == Some(removed_id))
            {
                self.close_context_menu();
            }
        }
        self.reset_after_structure_change();
        Some(removed)
    }

    fn reset_after_structure_change(&mut self) {
        self.interaction.clear();
        self.focusable_ids.clear();
        self.focusable_bounds.clear();
        self.placed.clear();
    }

    #[must_use]
    pub fn roots(&self) -> &[UiComponent] {
        &self.roots
    }

    pub fn find(&self, id: ComponentId) -> Option<&UiComponent> {
        self.roots.iter().find_map(|root| find_component(root, id))
    }

    pub fn find_mut(&mut self, id: ComponentId) -> Option<&mut UiComponent> {
        self.roots
            .iter_mut()
            .find_map(|root| find_component_mut(root, id))
    }

    /// Computes the full visible component tree in depth-first paint order.
    /// Child rectangles are produced by each parent's stack layout.
    pub fn layout(&mut self, bounds: Rect) -> &[PlacedComponent] {
        self.placed.clear();
        self.focusable_ids.clear();
        self.focusable_bounds.clear();

        let roots: Vec<_> = self
            .roots
            .iter()
            .filter(|node| node.visible)
            .cloned()
            .collect();
        let root_items: Vec<_> = roots.iter().map(|node| node.layout_item).collect();
        let root_rects = StackLayout::default().layout(bounds, &root_items);
        for (node, rect) in roots.iter().zip(root_rects) {
            self.place_node(node, rect, true);
        }
        self.placed.as_slice()
    }

    fn place_node(&mut self, node: &UiComponent, bounds: Rect, ancestors_enabled: bool) {
        let effectively_enabled = ancestors_enabled && node.enabled;
        let state = if node.is_focusable() && ancestors_enabled {
            let index = self.focusable_ids.len();
            self.focusable_ids.push(node.id);
            self.focusable_bounds.push(bounds);
            self.interaction.widget_state(index, true)
        } else if !effectively_enabled {
            WidgetState::Disabled
        } else {
            WidgetState::Normal
        };
        self.placed.push(PlacedComponent {
            id: node.id,
            bounds,
            state,
        });

        let children: Vec<_> = node.children.iter().filter(|child| child.visible).collect();
        if !children.is_empty() {
            let items: Vec<_> = children.iter().map(|child| child.layout_item).collect();
            let child_rects = node.child_layout.layout(bounds, &items);
            for (child, child_bounds) in children.into_iter().zip(child_rects) {
                self.place_node(child, child_bounds, effectively_enabled);
            }
        }
    }

    /// Updates pointer state and emits activation/focus events for visible controls.
    pub fn update_pointer(
        &mut self,
        bounds: Rect,
        pointer: Option<Point>,
        pressed: bool,
        released: bool,
    ) -> ComponentEvents {
        self.layout(bounds);
        if let (Some(point), Some(mut menu)) = (pointer, self.context_menu_state) {
            let mut hit = false;
            for depth in (1..=menu.submenu_depth).rev() {
                let popup = self.nested_submenu_bounds(bounds, menu, depth);
                if popup.contains(point) {
                    let row =
                        (u32::try_from(point.y).unwrap_or(0).saturating_sub(popup.y) / 28) as usize;
                    if let Some(item) = Self::context_menu_items_at_depth(
                        &self.active_context_menu_items,
                        menu,
                        depth,
                    )
                    .and_then(|items| items.get(row))
                    {
                        if item.enabled {
                            menu.submenu_path[dept…17800 tokens truncated… root = UiComponent::panel(ComponentId(10), StackLayout::new(LayoutDirection::Vertical))
            .with_children(vec![
                UiComponent::button(ComponentId(11), "Disabled")
                    .with_layout_item(LayoutItem::fixed(10))
                    .with_enabled(false),
                UiComponent::button(ComponentId(12), "Hidden")
                    .with_layout_item(LayoutItem::fixed(10))
                    .with_visible(false),
                UiComponent::button(ComponentId(13), "Visible")
                    .with_layout_item(LayoutItem::fixed(10)),
            ]);
        let mut tree = ComponentTree::new(vec![root]);
        let placed = tree.layout(Rect::new(0, 0, 40, 40));
        assert_eq!(placed.len(), 3);
        assert_eq!(placed[1].state, WidgetState::Disabled);
        assert_eq!(placed[2].id, ComponentId(13));
        assert_eq!(
            tree.focus_next(Rect::new(0, 0, 40, 40), false)
                .focus_changed,
            Some(ComponentId(13))
        );
    }

    #[test]
    fn component_values_can_be_updated_by_stable_id() {
        let mut tree = sample_tree();
        assert!(tree.find_mut(ComponentId(4)).unwrap().set_checked(true));
        assert!(tree
            .find_mut(ComponentId(2))
            .unwrap()
            .set_text("Preferences"));
        assert!(matches!(
            tree.find(ComponentId(4)).unwrap().kind(),
            ComponentKind::Checkbox { checked: true, .. }
        ));
        assert!(matches!(
            tree.find(ComponentId(2)).unwrap().kind(),
            ComponentKind::Label { text } if text == "Preferences"
        ));
    }

    #[test]
    fn runtime_enabled_and_visible_changes_reconcile_layout_and_focus() {
        let bounds = Rect::new(0, 0, 100, 60);
        let mut tree = ComponentTree::new(vec![
            UiComponent::button(ComponentId(101), "First").with_layout_item(LayoutItem::fixed(20)),
            UiComponent::button(ComponentId(102), "Second").with_layout_item(LayoutItem::fixed(20)),
        ]);
        assert_eq!(
            tree.focus_next(bounds, false).focus_changed,
            Some(ComponentId(101))
        );
        assert!(tree.set_enabled(ComponentId(101), false));
        assert_eq!(tree.focused_component(), None);
        assert_eq!(
            tree.focus_next(bounds, false).focus_changed,
            Some(ComponentId(102))
        );
        assert!(tree.set_visible(ComponentId(102), false));
        let remaining = tree.layout(bounds);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, ComponentId(101));
        assert_eq!(remaining[0].state, WidgetState::Disabled);
        assert_eq!(tree.focus_next(bounds, false).focus_changed, None);
        assert!(!tree.set_visible(ComponentId(102), false));
        assert!(!tree.set_enabled(ComponentId(999), true));
    }

    #[test]
    fn tree_value_setters_update_checkbox_label_and_text_input_state() {
        let mut tree = ComponentTree::new(vec![
            UiComponent::checkbox(ComponentId(103), "Enabled", false),
            UiComponent::label(ComponentId(104), "Old"),
            UiComponent::text_input(ComponentId(105), "Name", "before"),
        ]);
        assert!(tree.set_checked(ComponentId(103), true));
        assert!(tree.set_text(ComponentId(104), "New"));
        assert!(tree.set_text(ComponentId(105), "after"));
        assert_eq!(
            tree.text_input_state(ComponentId(105)).unwrap().text(),
            "after"
        );
        assert!(matches!(
            tree.find(ComponentId(103)).unwrap().kind(),
            ComponentKind::Checkbox { checked: true, .. }
        ));
        assert!(matches!(
            tree.find(ComponentId(104)).unwrap().kind(),
            ComponentKind::Label { text } if text == "New"
        ));
        assert!(matches!(
            tree.find(ComponentId(105)).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "after"
        ));
        assert!(!tree.set_checked(ComponentId(104), true));
        assert!(!tree.set_text(ComponentId(999), "missing"));
    }

    #[test]
    fn checkbox_toggles_on_pointer_and_keyboard_activation() {
        let bounds = Rect::new(10, 10, 100, 80);
        let mut tree = sample_tree();
        tree.update_pointer(bounds, Some(Point::new(20, 50)), true, false);
        let events = tree.update_pointer(bounds, Some(Point::new(20, 50)), false, true);
        assert_eq!(events.activated, Some(ComponentId(4)));
        assert!(matches!(
            tree.find(ComponentId(4)).unwrap().kind(),
            ComponentKind::Checkbox { checked: true, .. }
        ));
        tree.focus_next(bounds, false);
        tree.focus_next(bounds, false);
        tree.activate_focused(bounds);
        assert!(matches!(
            tree.find(ComponentId(4)).unwrap().kind(),
            ComponentKind::Checkbox { checked: false, .. }
        ));
    }

    #[test]
    fn text_input_editing_preserves_unicode_and_rejects_other_components() {
        let mut tree = ComponentTree::new(vec![UiComponent::text_input(
            ComponentId(30),
            "Type here",
            "café",
        )]);
        assert!(tree.insert_text(ComponentId(30), "🙂"));
        assert!(matches!(
            tree.find(ComponentId(30)).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "café🙂"
        ));
        assert!(tree.backspace_text(ComponentId(30)));
        assert!(matches!(
            tree.find(ComponentId(30)).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "café"
        ));
        assert!(!tree.insert_text(ComponentId(999), "x"));
    }

    #[test]
    fn cursor_and_selection_persist_between_text_edit_operations() {
        let id = ComponentId(31);
        let mut tree = ComponentTree::new(vec![UiComponent::text_input(id, "", "abcd")]);
        assert!(tree.move_text_cursor(id, false, false));
        assert!(tree.move_text_cursor(id, false, false));
        assert_eq!(tree.text_input_state(id).unwrap().cursor(), 2);
        assert!(tree.insert_text(id, "X"));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "abXcd"
        ));
        assert_eq!(tree.text_input_state(id).unwrap().cursor(), 3);
        assert!(tree.backspace_text(id));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "abcd"
        ));
        assert!(tree.select_all_text(id));
        assert!(tree.insert_text(id, "é🙂"));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "é🙂"
        ));
        assert_eq!(tree.text_input_state(id).unwrap().cursor(), "é🙂".len());
    }

    #[test]
    fn keyboard_events_route_to_focused_text_input() {
        let bounds = Rect::new(0, 0, 100, 40);
        let id = ComponentId(32);
        let mut tree = ComponentTree::new(vec![
            UiComponent::text_input(id, "Name", "AB").with_layout_item(LayoutItem::fixed(20)),
            UiComponent::button(ComponentId(33), "OK").with_layout_item(LayoutItem::fixed(20)),
        ]);
        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(id));
        assert_eq!(tree.focused_text_input(), Some(id));
        tree.handle_key(bounds, ComponentKey::ArrowLeft, false);
        assert!(tree.insert_text_focused("🙂"));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "A🙂B"
        ));
        tree.handle_key(bounds, ComponentKey::Backspace, false);
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "AB"
        ));
        assert_eq!(
            tree.handle_key(bounds, ComponentKey::Tab, false)
                .focus_changed,
            Some(ComponentId(33))
        );
        assert_eq!(tree.focused_text_input(), None);
    }

    #[test]
    fn external_text_updates_resynchronize_the_retained_cursor() {
        let id = ComponentId(34);
        let mut tree = ComponentTree::new(vec![UiComponent::text_input(id, "", "abc")]);
        tree.move_text_cursor(id, false, false);
        assert_eq!(tree.text_input_state(id).unwrap().cursor(), 2);
        assert!(tree.find_mut(id).unwrap().set_text("updated"));
        assert!(tree.insert_text(id, "!"));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "updated!"
        ));
        assert_eq!(
            tree.text_input_state(id).unwrap().cursor(),
            "updated!".len()
        );
    }

    #[test]
    fn platform_text_events_keep_ime_preedit_separate_until_commit() {
        let id = ComponentId(81);
        let mut tree = ComponentTree::new(vec![UiComponent::text_input(id, "", "hello")]);
        assert!(tree.handle_text_input_event(id, TextInputEvent::CompositionUpdate("に".into())));
        assert_eq!(tree.text_input_state(id).unwrap().composition(), Some("に"));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "hello"
        ));
        assert!(
            tree.handle_text_input_event(id, TextInputEvent::CompositionCommit("日本語".into()))
        );
        assert_eq!(tree.text_input_state(id).unwrap().composition(), None);
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value == "hello日本語"
        ));
        assert!(tree.handle_text_input_event(id, TextInputEvent::CompositionUpdate("x".into())));
        assert!(tree.handle_text_input_event(id, TextInputEvent::CompositionCancel));
        assert_eq!(tree.text_input_state(id).unwrap().composition(), None);
    }

    #[test]
    fn clipboard_selection_can_be_copied_and_cut() {
        let id = ComponentId(82);
        let mut tree = ComponentTree::new(vec![UiComponent::text_input(id, "", "hello")]);
        assert!(tree.select_all_text(id));
        assert_eq!(tree.copy_selected_text(id).as_deref(), Some("hello"));
        assert_eq!(tree.cut_selected_text(id).as_deref(), Some("hello"));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::TextInput { value, .. } if value.is_empty()
        ));
        assert_eq!(tree.copy_selected_text(id), None);
    }

    #[test]
    fn retained_combo_box_supports_focus_keyboard_open_highlight_and_selection() {
        let id = ComponentId(120);
        let bounds = Rect::new(0, 0, 160, 36);
        let mut tree = ComponentTree::new(vec![UiComponent::combo_box(
            id,
            vec!["First".into(), "Second".into(), "Third".into()],
        )
        .with_layout_item(LayoutItem::fixed(36))]);
        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(id));
        tree.handle_key(bounds, ComponentKey::Enter, false);
        assert!(tree.combo_states[&id].is_open());
        assert_eq!(tree.combo_states[&id].highlighted(), Some(0));
        tree.handle_key(bounds, ComponentKey::ArrowDown, false);
        assert_eq!(tree.combo_states[&id].highlighted(), Some(1));
        tree.handle_key(bounds, ComponentKey::Enter, false);
        assert_eq!(tree.combo_box_selected(id), Some(1));
        assert!(!tree.combo_states[&id].is_open());
    }

    #[test]
    fn retained_combo_box_escape_closes_without_changing_selection() {
        let id = ComponentId(121);
        let bounds = Rect::new(0, 0, 160, 36);
        let mut tree = ComponentTree::new(vec![UiComponent::combo_box(
            id,
            vec!["A".into(), "B".into()],
        )
        .with_layout_item(LayoutItem::fixed(36))]);
        tree.focus_next(bounds, false);
        tree.handle_key(bounds, ComponentKey::Enter, false);
        tree.handle_key(bounds, ComponentKey::ArrowDown, false);
        tree.handle_key(bounds, ComponentKey::Escape, false);
        assert_eq!(tree.combo_box_selected(id), None);
        assert!(!tree.combo_states[&id].is_open());
    }

    #[test]
    fn retained_tabs_support_focus_and_bounded_keyboard_navigation() {
        let id = ComponentId(122);
        let bounds = Rect::new(0, 0, 180, 36);
        let mut tree = ComponentTree::new(vec![UiComponent::tabs(
            id,
            vec!["General".into(), "Display".into(), "Input".into()],
        )
        .with_layout_item(LayoutItem::fixed(36))]);
        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(id));
        tree.handle_key(bounds, ComponentKey::ArrowRight, false);
        assert_eq!(tree.tabs_selected(id), Some(0));
        tree.handle_key(bounds, ComponentKey::ArrowRight, false);
        assert_eq!(tree.tabs_selected(id), Some(1));
        tree.handle_key(bounds, ComponentKey::ArrowLeft, false);
        assert_eq!(tree.tabs_selected(id), Some(0));
        tree.handle_key(bounds, ComponentKey::ArrowLeft, false);
        assert_eq!(tree.tabs_selected(id), Some(0));
    }

    #[test]
    fn retained_combo_box_selects_popup_rows_with_pointer() {
        let id = ComponentId(123);
        let bounds = Rect::new(0, 0, 160, 36);
        let mut tree = ComponentTree::new(vec![UiComponent::combo_box(
            id,
            vec!["First".into(), "Second".into(), "Third".into()],
        )
        .with_layout_item(LayoutItem::fixed(36))]);
        tree.focus_next(bounds, false);
        tree.handle_key(bounds, ComponentKey::Enter, false);

        let second_row = Point::new(20, 36 + 28 + 8);
        tree.update_pointer(bounds, Some(second_row), true, false);
        tree.update_pointer(bounds, Some(second_row), false, true);

        assert_eq!(tree.combo_box_selected(id), Some(1));
        assert!(!tree.combo_states[&id].is_open());
    }

    #[test]
    fn retained_tabs_select_the_clicked_tab_with_pointer() {
        let id = ComponentId(124);
        let bounds = Rect::new(0, 0, 180, 36);
        let mut tree = ComponentTree::new(vec![UiComponent::tabs(
            id,
            vec!["General".into(), "Display".into(), "Input".into()],
        )
        .with_layout_item(LayoutItem::fixed(36))]);

        let third_tab = Point::new(150, 18);
        tree.update_pointer(bounds, Some(third_tab), true, false);
        let events = tree.update_pointer(bounds, Some(third_tab), false, true);

        assert_eq!(events.activated, Some(id));
        assert_eq!(tree.tabs_selected(id), Some(2));
    }

    #[test]
    fn retained_switch_toggles_with_pointer_and_keyboard() {
        let id = ComponentId(130);
        let bounds = Rect::new(0, 0, 120, 32);
        let mut tree = ComponentTree::new(vec![
            UiComponent::switch(id, "Wi-Fi", false).with_layout_item(LayoutItem::fixed(32))
        ]);
        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(id));
        tree.activate_focused(bounds);
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::Switch { enabled: true, .. }
        ));
        tree.update_pointer(bounds, Some(Point::new(10, 16)), true, false);
        tree.update_pointer(bounds, Some(Point::new(10, 16)), false, true);
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::Switch { enabled: false, .. }
        ));
    }

    #[test]
    fn retained_radio_buttons_are_exclusive_within_their_group() {
        let first = ComponentId(131);
        let second = ComponentId(132);
        let bounds = Rect::new(0, 0, 120, 64);
        let mut tree = ComponentTree::new(vec![
            UiComponent::radio_button(first, "One", 7, true)
                .with_layout_item(LayoutItem::fixed(32)),
            UiComponent::radio_button(second, "Two", 7, false)
                .with_layout_item(LayoutItem::fixed(32)),
        ]);
        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(first));
        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(second));
        tree.activate_focused(bounds);
        assert!(matches!(
            tree.find(first).unwrap().kind(),
            ComponentKind::RadioButton {
                selected: false,
                ..
            }
        ));
        assert!(matches!(
            tree.find(second).unwrap().kind(),
            ComponentKind::RadioButton { selected: true, .. }
        ));
    }

    #[test]
    fn retained_slider_clamps_values_and_supports_keyboard_and_pointer() {
        let id = ComponentId(133);
        let bounds = Rect::new(0, 0, 100, 32);
        let mut tree = ComponentTree::new(vec![
            UiComponent::slider(id, "Volume", 0, 10, 4).with_layout_item(LayoutItem::fixed(32))
        ]);
        assert_eq!(tree.slider_value(id), Some(4));
        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(id));
        tree.handle_key(bounds, ComponentKey::ArrowRight, false);
        assert_eq!(tree.slider_value(id), Some(5));
        assert!(tree.set_slider_value(id, 99));
        assert_eq!(tree.slider_value(id), Some(10));
        tree.update_pointer(bounds, Some(Point::new(0, 16)), true, false);
        tree.update_pointer(bounds, Some(Point::new(0, 16)), false, true);
        assert_eq!(tree.slider_value(id), Some(0));
    }

    #[test]
    fn retained_slider_tracks_pointer_motion_while_dragging() {
        let id = ComponentId(134);
        let bounds = Rect::new(0, 0, 100, 32);
        let mut tree =
            ComponentTree::new(vec![UiComponent::slider(id, "Volume", 0, 100, 0)
                .with_layout_item(LayoutItem::fixed(32))]);

        tree.update_pointer(bounds, Some(Point::new(0, 16)), true, false);
        assert_eq!(tree.slider_value(id), Some(0));

        tree.update_pointer(bounds, Some(Point::new(50, 16)), false, false);
        assert_eq!(tree.slider_value(id), Some(50));

        tree.update_pointer(bounds, Some(Point::new(90, 16)), false, false);
        assert_eq!(tree.slider_value(id), Some(90));

        tree.update_pointer(bounds, Some(Point::new(90, 16)), false, true);
        assert_eq!(tree.slider_value(id), Some(90));
    }

    #[test]
    fn retained_list_view_supports_pointer_selection_keyboard_and_scroll() {
        let id = ComponentId(135);
        let bounds = Rect::new(0, 0, 100, 64);
        let mut tree = ComponentTree::new(vec![UiComponent::list_view(
            id,
            vec![
                "One".into(),
                "Two".into(),
                "Three".into(),
                "Four".into(),
                "Five".into(),
            ],
            16,
        )
        .with_layout_item(LayoutItem::fixed(64))]);

        assert_eq!(tree.focus_next(bounds, false).focus_changed, Some(id));
        tree.update_pointer(bounds, Some(Point::new(10, 40)), true, false);
        tree.update_pointer(bounds, Some(Point::new(10, 40)), false, true);
        assert_eq!(tree.list_view_selected(id), Some(2));

        tree.handle_key(bounds, ComponentKey::ArrowUp, false);
        assert_eq!(tree.list_view_selected(id), Some(1));

        assert!(tree.set_list_view_scroll(id, 16));
        tree.update_pointer(bounds, Some(Point::new(10, 8)), true, false);
        tree.update_pointer(bounds, Some(Point::new(10, 8)), false, true);
        assert_eq!(tree.list_view_selected(id), Some(1));
    }

    #[test]
    fn retained_list_view_scrolls_under_pointer_and_clamps_to_content() {
        let id = ComponentId(136);
        let bounds = Rect::new(0, 0, 100, 48);
        let mut tree = ComponentTree::new(vec![UiComponent::list_view(
            id,
            (0..10).map(|index| format!("Row {index}")).collect(),
            16,
        )
        .with_layout_item(LayoutItem::fixed(48))]);
        tree.layout(bounds);

        assert!(tree.scroll_list_view_at(Point::new(10, 20), 32));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::ListView {
                scroll_offset: 32,
                ..
            }
        ));
        assert!(tree.scroll_list_view_at(Point::new(10, 20), -16));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::ListView {
                scroll_offset: 16,
                ..
            }
        ));
        assert!(tree.scroll_list_view_at(Point::new(10, 20), i32::MAX));
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            ComponentKind::ListView {
                scroll_offset: 112,
                ..
            }
        ));
        assert!(!tree.scroll_list_view_at(Point::new(150, 20), 16));
    }

    #[test]
    fn context_menu_shortcuts_match_modifiers_and_activate_submenu_commands() {
        let child = ContextMenuItem::new(902, "Save as").with_shortcut(
            ContextMenuShortcut::new('s')
                .with_control(true)
                .with_shift(true),
        );
        let parent = ContextMenuItem::new(901, "File").with_submenu(vec![child]);
        let mut tree = ComponentTree::new(Vec::new());
        tree.set_context_menu(vec![parent]);

        assert!(tree
            .activate_context_menu_shortcut('s', true, false, false)
            .is_none());
        let events = tree
            .activate_context_menu_shortcut('S', true, false, true)
            .unwrap();
        assert_eq!(events.context_menu_action, Some(902));
        assert_eq!(events.context_menu_target, None);
        assert!(!tree.context_menu_is_open());
    }

    #[test]
    fn context_menu_shortcuts_find_commands_at_arbitrary_submenu_depth() {
        let nested = ContextMenuItem::new(905, "Export").with_submenu(vec![ContextMenuItem::new(
            906,
            "As archive",
        )
        .with_submenu(vec![ContextMenuItem::new(907, "Compressed")
            .with_shortcut(ContextMenuShortcut::new('e').with_control(true))])]);
        let mut tree = ComponentTree::new(Vec::new());
        tree.set_context_menu(vec![nested]);

        let events = tree
            .activate_context_menu_shortcut('e', true, false, false)
            .unwrap();
        assert_eq!(events.context_menu_action, Some(907));
    }

    #[test]
    fn context_menu_item_label_includes_shortcut_and_submenu_indicator() {
        let item = ContextMenuItem::new(908, "Save")
            .with_shortcut(
                ContextMenuShortcut::new('s')
                    .with_control(true)
                    .with_shift(true),
            )
            .with_submenu(vec![ContextMenuItem::new(909, "More")]);
        assert_eq!(
            super::format_context_menu_item_label(&item),
            "Save  >    Ctrl+Shift+S"
        );
    }

    #[test]
    fn disabled_context_menu_shortcuts_do_not_activate() {
        let item = ContextMenuItem::new(903, "Unavailable")
            .with_shortcut(ContextMenuShortcut::new('u').with_control(true))
            .with_enabled(false);
        let mut tree = ComponentTree::new(Vec::new());
        tree.set_context_menu(vec![item]);
        assert!(tree
            .activate_context_menu_shortcut('u', true, false, false)
            .is_none());
    }

    #[test]
    fn hidden_children_do_not_consume_layout_space() {
        let root = UiComponent::panel(ComponentId(20), StackLayout::new(LayoutDirection::Vertical))
            .with_children(vec![
                UiComponent::button(ComponentId(21), "Hidden")
                    .with_layout_item(LayoutItem::fixed(10))
                    .with_visible(false),
                UiComponent::button(ComponentId(22), "Visible")
                    .with_layout_item(LayoutItem::fixed(10)),
            ]);
        let mut tree = ComponentTree::new(vec![root]);
        let placed = tree.layout(Rect::new(0, 0, 50, 30));
        assert_eq!(placed.len(), 2);
        assert_eq!(placed[1].id, ComponentId(22));
        assert_eq!(placed[1].bounds.y, 0);
    }
}
