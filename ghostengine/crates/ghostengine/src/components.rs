//! Retained-mode component tree for application interfaces.
//!
//! The tree owns component identity, hierarchy, layout participation, and
//! interaction dispatch. Rendering remains explicit: applications can draw
//! each returned rectangle with GhostEngine's existing CPU widget primitives.

use crate::{
    InteractionEvents, LayoutItem, Point, Rect, StackLayout, UiInteraction, WidgetState,
};

/// Stable caller-assigned identity for a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(pub u64);

/// The semantic role and user-owned data of a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentKind {
    Panel,
    Button { label: String },
    Checkbox { label: String, checked: bool },
    TextInput { placeholder: String, value: String },
    Label { text: String },
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
    children: Vec<Self>,
}

impl UiComponent {
    #[must_use]
    pub fn panel(id: ComponentId, layout: StackLayout) -> Self {
        Self::new(id, ComponentKind::Panel).with_child_layout(layout)
    }

    #[must_use]
    pub fn button(id: ComponentId, label: impl Into<String>) -> Self {
        Self::new(id, ComponentKind::Button { label: label.into() })
    }

    #[must_use]
    pub fn checkbox(
        id: ComponentId,
        label: impl Into<String>,
        checked: bool,
    ) -> Self {
        Self::new(id, ComponentKind::Checkbox {
            label: label.into(),
            checked,
        })
    }

    #[must_use]
    pub fn text_input(
        id: ComponentId,
        placeholder: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self::new(id, ComponentKind::TextInput {
            placeholder: placeholder.into(),
            value: value.into(),
        })
    }

    #[must_use]
    pub fn label(id: ComponentId, text: impl Into<String>) -> Self {
        Self::new(id, ComponentKind::Label { text: text.into() })
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
            )
    }
}

/// A component's computed geometry and visual interaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedComponent {
    pub id: ComponentId,
    pub bounds: Rect,
    pub state: WidgetState,
}

/// Component-level events emitted by pointer or keyboard interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComponentEvents {
    pub activated: Option<ComponentId>,
    pub focus_changed: Option<ComponentId>,
}

/// Retained component hierarchy with layout and interaction dispatch.
#[derive(Debug, Clone, Default)]
pub struct ComponentTree {
    roots: Vec<UiComponent>,
    interaction: UiInteraction,
    focusable_ids: Vec<ComponentId>,
    focusable_bounds: Vec<Rect>,
    placed: Vec<PlacedComponent>,
}

impl ComponentTree {
    #[must_use]
    pub fn new(roots: Vec<UiComponent>) -> Self {
        Self {
            roots,
            interaction: UiInteraction::new(),
            focusable_ids: Vec::new(),
            focusable_bounds: Vec::new(),
            placed: Vec::new(),
        }
    }

    #[must_use]
    pub fn roots(&self) -> &[UiComponent] {
        &self.roots
    }

    pub fn find(&self, id: ComponentId) -> Option<&UiComponent> {
        self.roots.iter().find_map(|root| find_component(root, id))
    }

    pub fn find_mut(&mut self, id: ComponentId) -> Option<&mut UiComponent> {
        self.roots.iter_mut().find_map(|root| find_component_mut(root, id))
    }

    /// Computes the full visible component tree in depth-first paint order.
    /// Child rectangles are produced by each parent's stack layout.
    pub fn layout(&mut self, bounds: Rect) -> &[PlacedComponent] {
        self.placed.clear();
        self.focusable_ids.clear();
        self.focusable_bounds.clear();

        let roots = self.roots.clone();
        let root_items: Vec<_> = roots.iter().map(|node| node.layout_item).collect();
        let root_rects = StackLayout::default().layout(bounds, &root_items);
        for (node, rect) in roots.iter().zip(root_rects) {
            self.place_node(node, rect);
        }
        self.placed.as_slice()
    }

    fn place_node(&mut self, node: &UiComponent, bounds: Rect) {
        if !node.visible {
            return;
        }
        let state = if node.is_focusable() {
            let index = self.focusable_ids.len();
            self.focusable_ids.push(node.id);
            self.focusable_bounds.push(bounds);
            self.interaction.widget_state(index, true)
        } else if !node.enabled {
            WidgetState::Disabled
        } else {
            WidgetState::Normal
        };
        self.placed.push(PlacedComponent {
            id: node.id,
            bounds,
            state,
        });

        if !node.children.is_empty() {
            let items: Vec<_> = node.children.iter().map(|child| child.layout_item).collect();
            let child_rects = node.child_layout.layout(bounds, &items);
            for (child, child_bounds) in node.children.iter().zip(child_rects) {
                self.place_node(child, child_bounds);
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
        let enabled = vec![true; self.focusable_ids.len()];
        let events = self.interaction.update(
            pointer,
            pressed,
            released,
            &enabled,
            &self.focusable_bounds,
        );
        self.translate_events(events)
    }

    /// Moves focus through visible enabled controls in depth-first tab order.
    pub fn focus_next(&mut self, bounds: Rect, reverse: bool) -> ComponentEvents {
        self.layout(bounds);
        let enabled = vec![true; self.focusable_ids.len()];
        let events = self.interaction.focus_next(&enabled, reverse);
        self.translate_events(events)
    }

    /// Activates the currently focused component (typically Enter or Space).
    pub fn activate_focused(&mut self, bounds: Rect) -> ComponentEvents {
        self.layout(bounds);
        let enabled = vec![true; self.focusable_ids.len()];
        let events = self
            .interaction
            .activate_focused(&enabled, &self.focusable_bounds);
        self.translate_events(events)
    }

    /// Returns the last computed component placements.
    #[must_use]
    pub fn placed(&self) -> &[PlacedComponent] {
        &self.placed
    }

    /// Clears hover, pointer capture, and keyboard focus.
    pub fn clear_interaction(&mut self) {
        self.interaction.clear();
    }

    fn translate_events(&self, events: InteractionEvents) -> ComponentEvents {
        ComponentEvents {
            activated: events.clicked.and_then(|index| self.focusable_ids.get(index).copied()),
            focus_changed: events
                .focus_changed
                .and_then(|index| self.focusable_ids.get(index).copied()),
        }
    }
}

fn find_component(node: &UiComponent, id: ComponentId) -> Option<&UiComponent> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|child| find_component(child, id))
}

fn find_component_mut(node: &mut UiComponent, id: ComponentId) -> Option<&mut UiComponent> {
    if node.id == id {
        return Some(node);
    }
    node.children
        .iter_mut()
        .find_map(|child| find_component_mut(child, id))
}

#[cfg(test)]
mod tests {
    use super::{ComponentId, ComponentKind, ComponentTree, UiComponent};
    use crate::{Insets, LayoutDirection, LayoutItem, Point, Rect, StackLayout, WidgetState};

    fn sample_tree() -> ComponentTree {
        let panel = UiComponent::panel(
            ComponentId(1),
            StackLayout::new(LayoutDirection::Vertical)
                .with_padding(Insets::all(2))
                .with_gap(2),
        )
        .with_layout_item(LayoutItem::flexible())
        .with_children(vec![
            UiComponent::label(ComponentId(2), "Settings")
                .with_layout_item(LayoutItem::fixed(12)),
            UiComponent::button(ComponentId(3), "Apply")
                .with_layout_item(LayoutItem::fixed(20)),
            UiComponent::checkbox(ComponentId(4), "Enable", false)
                .with_layout_item(LayoutItem::fixed(20)),
        ]);
        ComponentTree::new(vec![panel])
    }

    #[test]
    fn tree_layout_computes_nested_geometry_and_paint_order() {
        let mut tree = sample_tree();
        let placed = tree.layout(Rect::new(10, 10, 100, 80));
        assert_eq!(placed.len(), 4);
        assert_eq!(placed[0].id, ComponentId(1));
        assert_eq!(placed[1].id, ComponentId(2));
        assert_eq!(placed[1].bounds, Rect::new(12, 12, 96, 12));
        assert_eq!(placed[2].bounds, Rect::new(12, 26, 96, 20));
        assert_eq!(placed[3].bounds, Rect::new(12, 48, 96, 20));
    }

    #[test]
    fn pointer_clicks_are_dispatched_to_component_identity() {
        let mut tree = sample_tree();
        let bounds = Rect::new(10, 10, 100, 80);
        tree.update_pointer(bounds, Some(Point::new(20, 30)), true, false);
        let events = tree.update_pointer(bounds, Some(Point::new(20, 30)), false, true);
        assert_eq!(events.activated, Some(ComponentId(3)));
        assert_eq!(events.focus_changed, None);
    }

    #[test]
    fn keyboard_tab_and_activation_dispatch_to_component_identity() {
        let mut tree = sample_tree();
        let bounds = Rect::new(10, 10, 100, 80);
        assert_eq!(
            tree.focus_next(bounds, false).focus_changed,
            Some(ComponentId(3))
        );
        assert_eq!(
            tree.activate_focused(bounds).activated,
            Some(ComponentId(3))
        );
        assert_eq!(
            tree.focus_next(bounds, false).focus_changed,
            Some(ComponentId(4))
        );
    }

    #[test]
    fn disabled_and_hidden_components_are_excluded_from_layout_and_tab_order() {
        let root = UiComponent::panel(
            ComponentId(10),
            StackLayout::new(LayoutDirection::Vertical),
        )
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
            tree.focus_next(Rect::new(0, 0, 40, 40), false).focus_changed,
            Some(ComponentId(13))
        );
    }

    #[test]
    fn component_values_can_be_updated_by_stable_id() {
        let mut tree = sample_tree();
        assert!(tree.find_mut(ComponentId(4)).unwrap().set_checked(true));
        assert!(tree.find_mut(ComponentId(2)).unwrap().set_text("Preferences"));
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
    fn hidden_children_do_not_consume_layout_space() {
        let root = UiComponent::panel(
            ComponentId(20),
            StackLayout::new(LayoutDirection::Vertical),
        )
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
