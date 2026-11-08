//! Retained-mode component tree for application interfaces.
//!
//! The tree owns component identity, hierarchy, layout participation, and
//! interaction dispatch. Components render directly into GhostEngine's CPU framebuffer.

use std::collections::HashMap;

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

/// Component-level events emitted by pointer or keyboard interaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentEvents {
    pub activated: Option<ComponentId>,
    pub focus_changed: Option<ComponentId>,
    /// Host clipboard operation requested by a keyboard shortcut.
    pub clipboard_action: Option<ClipboardAction>,
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
        let enabled = vec![true; self.focusable_ids.len()];
        // Popup rows live outside the combo box's layout rectangle, so route
        // their pointer releases before normal component hit testing.
        if released {
            if let Some(point) = pointer {
                let placements = self.placed.clone();
                for placement in placements {
                    let Some(ComponentKind::ComboBox { items }) = self
                        .find(placement.id)
                        .map(|component| component.kind().clone())
                    else {
                        continue;
                    };
                    let Some(state) = self.combo_states.get_mut(&placement.id) else {
                        continue;
                    };
                    if !state.is_open() {
                        continue;
                    }
                    let popup_top = placement.bounds.y.saturating_add(placement.bounds.height);
                    let popup_height = (items.len() as u32).saturating_mul(28);
                    let popup = Rect::new(
                        placement.bounds.x,
                        popup_top,
                        placement.bounds.width,
                        popup_height,
                    );
                    if popup.contains(point) {
                        let index = ((point.y as u32 - popup_top) / 28) as usize;
                        state.select(Some(index), items.len());
                        state.close();
                        return ComponentEvents::default();
                    }
                }
            }
        }
        let captured_before_update = self.interaction.captured();
        let events =
            self.interaction
                .update(pointer, pressed, released, &enabled, &self.focusable_bounds);
        let translated = self.dispatch_events(events);
        if released {
            if let (Some(id), Some(point)) = (translated.activated, pointer) {
                let list_config = self.find(id).and_then(|component| match component.kind() {
                    ComponentKind::ListView {
                        items,
                        row_height,
                        scroll_offset,
                    } => Some((items.len(), *row_height, *scroll_offset)),
                    _ => None,
                });
                if let (Some((item_count, row_height, scroll_offset)), Some(rect)) = (
                    list_config,
                    self.placed
                        .iter()
                        .find(|placement| placement.id == id)
                        .map(|p| p.bounds),
                ) {
                    self.list_states.entry(id).or_default().select_at(
                        rect,
                        point,
                        scroll_offset,
                        row_height,
                        item_count,
                    );
                }
            }
        }

        // Keep updating a slider for every pointer-move frame while its
        // pointer capture is held, not only on the final click/release.
        let captured_slider =
            captured_before_update.and_then(|index| self.focusable_ids.get(index).copied());
        let activated_slider = translated.activated;
        if let (Some(id), Some(point)) = (captured_slider.or(activated_slider), pointer) {
            if let Some(rect) = self
                .placed
                .iter()
                .find(|placement| placement.id == id)
                .map(|placement| placement.bounds)
            {
                let range = self.find(id).and_then(|component| match component.kind() {
                    ComponentKind::Slider { min, max, .. } => Some((*min, *max)),
                    _ => None,
                });
                if let Some((min, max)) = range {
                    let travel = rect.width.max(1);
                    let relative = (point.x.max(0) as u32).saturating_sub(rect.x).min(travel);
                    let value = if max == min {
                        min
                    } else {
                        min.saturating_add(
                            ((u64::from(max - min) * u64::from(relative)) / u64::from(travel))
                                as u32,
                        )
                    };
                    self.set_slider_value(id, value);
                }
            }
        }
        if released {
            if let (Some(id), Some(point)) = (translated.activated, pointer) {
                let kind = self.find(id).map(|component| component.kind().clone());
                if let Some(ComponentKind::Tabs { labels }) = kind {
                    if !labels.is_empty() {
                        if let Some(placement) =
                            self.placed.iter().find(|placement| placement.id == id)
                        {
                            if placement.bounds.contains(point) {
                                let relative_x = point.x as u32 - placement.bounds.x;
                                let tab_width = placement.bounds.width.max(1) as u64;
                                let index = (u64::from(relative_x) * labels.len() as u64
                                    / tab_width)
                                    as usize;
                                self.tabs_states
                                    .entry(id)
                                    .or_default()
                                    .select(index.min(labels.len() - 1), labels.len());
                            }
                        }
                    }
                }
            }
        }
        translated
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
        self.dispatch_events(events)
    }

    /// Paints all visible components into the supplied CPU framebuffer.
    pub fn render(
        &mut self,
        target: &mut Framebuffer,
        bounds: Rect,
        font: &FontFace,
        pixel_size: f32,
        theme: &UiTheme,
    ) {
        let text_input_ids: Vec<_> = self.text_states.keys().copied().collect();
        for id in text_input_ids {
            self.sync_text_state(id);
        }
        self.layout(bounds);
        let placements = self.placed.clone();
        let focused = self.interaction.focused();
        let focusable = self.focusable_ids.clone();
        for placement in placements {
            let Some(component) = self.find(placement.id) else {
                continue;
            };
            match component.kind() {
                ComponentKind::Panel => draw_panel(target, placement.bounds, 6, theme),
                ComponentKind::Button { label } => draw_button(
                    target,
                    font,
                    placement.bounds,
                    label,
                    pixel_size,
                    placement.state,
                    theme,
                ),
                ComponentKind::Checkbox { label, checked } => {
                    draw_checkbox(target, placement.bounds, *checked, placement.state, theme);
                    let x = placement.bounds.x.saturating_add(30);
                    let baseline = i64::from(placement.bounds.y)
                        + (i64::from(placement.bounds.height)
                            + font.line_height(pixel_size).round() as i64)
                            / 2;
                    if let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(baseline)) {
                        draw_label_clipped(
                            target,
                            font,
                            label,
                            Point::new(x, y),
                            pixel_size,
                            theme.text,
                            placement.bounds,
                        );
                    }
                }
                ComponentKind::Switch { label, enabled } => {
                    draw_switch(target, placement.bounds, *enabled, placement.state, theme);
                    let x = placement
                        .bounds
                        .x
                        .saturating_add(placement.bounds.height.min(24) * 2 + 10);
                    let baseline = i64::from(placement.bounds.y)
                        + (i64::from(placement.bounds.height)
                            + font.line_height(pixel_size).round() as i64)
                            / 2;
                    if let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(baseline)) {
                        draw_label_clipped(
                            target,
                            font,
                            label,
                            Point::new(x, y),
                            pixel_size,
                            theme.text,
                            placement.bounds,
                        );
                    }
                }
                ComponentKind::RadioButton {
                    label, selected, ..
                } => {
                    draw_radio_button(
                        target,
                        font,
                        placement.bounds,
                        label,
                        *selected,
                        pixel_size,
                        placement.state,
                        theme,
                    );
                }
                ComponentKind::Slider {
                    min, max, value, ..
                } => {
                    draw_slider(
                        target,
                        placement.bounds,
                        *value,
                        *min,
                        *max,
                        placement.state,
                        theme,
                    );
                }
                ComponentKind::TextInput { placeholder, .. } => {
                    let Some(state) = self.text_states.get(&placement.id) else {
                        continue;
                    };
                    let is_focused = focusable
                        .iter()
                        .position(|id| *id == placement.id)
                        .is_some_and(|index| Some(index) == focused);
                    draw_text_input(
                        target,
                        font,
                        placement.bounds,
                        state,
                        TextInputOptions {
                            placeholder,
                            pixel_size,
                            focused: is_focused,
                            theme,
                        },
                    );
                }
                ComponentKind::Label { text } => {
                    let baseline = i64::from(placement.bounds.y)
                        + (font.line_height(pixel_size) * 0.78).round() as i64;
                    if let (Ok(x), Ok(y)) =
                        (i32::try_from(placement.bounds.x), i32::try_from(baseline))
                    {
                        draw_label_clipped(
                            target,
                            font,
                            text,
                            Point::new(x, y),
                            pixel_size,
                            theme.text,
                            placement.bounds,
                        );
                    }
                }
                ComponentKind::ComboBox { items } => {
                    let labels: Vec<_> = items.iter().map(String::as_str).collect();
                    let state = self
                        .combo_states
                        .get(&placement.id)
                        .copied()
                        .unwrap_or_default();
                    draw_combo_box(
                        target,
                        font,
                        placement.bounds,
                        &labels,
                        state,
                        pixel_size,
                        28,
                        theme,
                    );
                }
                ComponentKind::ListView {
                    items,
                    row_height,
                    scroll_offset,
                } => {
                    let labels: Vec<_> = items.iter().map(String::as_str).collect();
                    let state = self
                        .list_states
                        .get(&placement.id)
                        .copied()
                        .unwrap_or_default();
                    crate::draw_list(
                        target,
                        font,
                        placement.bounds,
                        &labels,
                        state,
                        ListOptions {
                            scroll_offset: *scroll_offset,
                            row_height: *row_height,
                            pixel_size,
                            theme,
                        },
                    );
                }
                ComponentKind::Tabs { labels } => {
                    let labels: Vec<_> = labels.iter().map(String::as_str).collect();
                    let state = self
                        .tabs_states
                        .get(&placement.id)
                        .copied()
                        .unwrap_or_default();
                    draw_tabs(
                        target,
                        font,
                        placement.bounds,
                        &labels,
                        state,
                        pixel_size,
                        theme,
                    );
                }
            }
        }

        // Tooltips are painted last so they remain visible above every widget.
        if let Some(hovered_id) = self
            .interaction
            .hovered()
            .and_then(|index| self.focusable_ids.get(index).copied())
        {
            let tooltip = self
                .find(hovered_id)
                .and_then(|component| component.tooltip.as_deref().map(str::to_owned));
            let anchor = self
                .placed
                .iter()
                .find(|placement| placement.id == hovered_id)
                .map(|placement| placement.bounds);
            if let (Some(text), Some(anchor)) = (tooltip, anchor) {
                let metrics = font.measure(&text, pixel_size);
                let line_height = font.line_height(pixel_size).ceil().max(1.0) as u32;
                let width = (metrics.width.ceil() as u32)
                    .saturating_add(16)
                    .min(target.width());
                let height = line_height.saturating_add(10).min(target.height());
                if width > 0 && height > 0 {
                    let max_x = target.width().saturating_sub(width);
                    let x = anchor.x.min(max_x);
                    let below = anchor.y.saturating_add(anchor.height).saturating_add(6);
                    let y = if below.saturating_add(height) <= target.height() {
                        below
                    } else {
                        anchor.y.saturating_sub(height.saturating_add(6))
                    }
                    .min(target.height().saturating_sub(height));
                    let tooltip_bounds = Rect::new(x, y, width, height);
                    draw_panel(target, tooltip_bounds, 4, theme);
                    let baseline = i64::from(y) + (i64::from(height) + i64::from(line_height)) / 2;
                    if let (Ok(text_x), Ok(text_y)) =
                        (i32::try_from(x.saturating_add(8)), i32::try_from(baseline))
                    {
                        draw_label_clipped(
                            target,
                            font,
                            &text,
                            Point::new(text_x, text_y),
                            pixel_size,
                            theme.text,
                            tooltip_bounds,
                        );
                    }
                }
            }
        }
    }

    /// Returns the selected row index of a retained list view.
    #[must_use]
    pub fn list_view_selected(&self, id: ComponentId) -> Option<usize> {
        self.list_states.get(&id).and_then(|state| state.selected())
    }

    /// Updates the pixel scroll offset of a retained list view.
    pub fn set_list_view_scroll(&mut self, id: ComponentId, offset: u32) -> bool {
        let Some(ComponentKind::ListView { scroll_offset, .. }) =
            self.find_mut(id).map(|component| &mut component.kind)
        else {
            return false;
        };
        *scroll_offset = offset;
        true
    }

    /// Returns the selected item index of a combo box.
    #[must_use]
    pub fn combo_box_selected(&self, id: ComponentId) -> Option<usize> {
        self.combo_states
            .get(&id)
            .and_then(|state| state.selected())
    }

    /// Returns the selected tab index of a tab strip.
    #[must_use]
    pub fn tabs_selected(&self, id: ComponentId) -> Option<usize> {
        self.tabs_states.get(&id).and_then(|state| state.selected())
    }

    /// Returns the stable ID of the currently focused visible control.
    #[must_use]
    pub fn focused_component(&self) -> Option<ComponentId> {
        self.interaction
            .focused()
            .and_then(|index| self.focusable_ids.get(index).copied())
    }

    /// Returns the focused text input, if keyboard focus belongs to one.
    #[must_use]
    pub fn focused_text_input(&self) -> Option<ComponentId> {
        let id = self.focused_component()?;
        matches!(
            self.find(id).map(UiComponent::kind),
            Some(ComponentKind::TextInput { .. })
        )
        .then_some(id)
    }

    /// Applies committed text or IME composition from a platform text service.
    pub fn handle_text_input_event(&mut self, id: ComponentId, event: TextInputEvent) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        match event {
            TextInputEvent::Insert(text) => state.insert_text(&text),
            TextInputEvent::CompositionUpdate(text) => state.set_composition(Some(text)),
            TextInputEvent::CompositionCommit(text) => {
                state.cancel_composition();
                state.insert_text(&text);
            }
            TextInputEvent::CompositionCancel => state.cancel_composition(),
        }
        self.commit_text_state(id)
    }

    /// Copies the active selection without modifying the input.
    pub fn copy_selected_text(&mut self, id: ComponentId) -> Option<String> {
        self.sync_text_state(id).then_some(())?;
        self.text_states
            .get(&id)?
            .selected_text()
            .map(str::to_owned)
    }

    /// Deletes the active selection and returns its former contents for the host clipboard.
    pub fn cut_selected_text(&mut self, id: ComponentId) -> Option<String> {
        let selected = self.copy_selected_text(id)?;
        self.text_states.get_mut(&id)?.delete_selection();
        self.commit_text_state(id).then_some(selected)
    }

    /// Reads the retained editing state for a text input.
    #[must_use]
    pub fn text_input_state(&self, id: ComponentId) -> Option<&TextInputState> {
        self.text_states.get(&id)
    }

    /// Inserts text, replacing any active selection.
    pub fn insert_text(&mut self, id: ComponentId, text: &str) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        state.insert_text(text);
        self.commit_text_state(id)
    }

    /// Deletes the previous grapheme, or active selection.
    pub fn backspace_text(&mut self, id: ComponentId) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        state.backspace();
        self.commit_text_state(id)
    }

    /// Deletes the next grapheme, or active selection.
    pub fn delete_text(&mut self, id: ComponentId) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        state.delete_forward();
        self.commit_text_state(id)
    }

    /// Moves the cursor by one grapheme, optionally extending selection.
    pub fn move_text_cursor(
        &mut self,
        id: ComponentId,
        right: bool,
        extend_selection: bool,
    ) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        if right {
            state.move_right(extend_selection);
        } else {
            state.move_left(extend_selection);
        }
        true
    }

    /// Moves the cursor to the beginning or end of the line.
    pub fn move_text_cursor_home_end(
        &mut self,
        id: ComponentId,
        end: bool,
        extend_selection: bool,
    ) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        if end {
            state.move_end(extend_selection);
        } else {
            state.move_home(extend_selection);
        }
        true
    }

    /// Moves the cursor by one Unicode word, optionally extending selection.
    pub fn move_text_word_cursor(
        &mut self,
        id: ComponentId,
        right: bool,
        extend_selection: bool,
    ) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        if right {
            state.move_word_right(extend_selection);
        } else {
            state.move_word_left(extend_selection);
        }
        true
    }

    /// Deletes the word before or after the cursor.
    pub fn delete_word_text(&mut self, id: ComponentId, forward: bool) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        if forward {
            state.delete_word_forward();
        } else {
            state.delete_word_backward();
        }
        self.commit_text_state(id)
    }

    /// Selects all text in the specified input.
    pub fn select_all_text(&mut self, id: ComponentId) -> bool {
        if !self.sync_text_state(id) {
            return false;
        }
        let Some(state) = self.text_states.get_mut(&id) else {
            return false;
        };
        state.select_all();
        true
    }

    /// Inserts text into the currently focused text input.
    pub fn insert_text_focused(&mut self, text: &str) -> bool {
        self.focused_text_input()
            .is_some_and(|id| self.insert_text(id, text))
    }

    /// Routes a discrete key to focus navigation, activation, or text editing.
    pub fn handle_key(&mut self, bounds: Rect, key: ComponentKey, shift: bool) -> ComponentEvents {
        if let Some(id) = self.focused_component() {
            let kind = self.find(id).map(|component| component.kind().clone());
            match kind {
                Some(ComponentKind::ComboBox { items }) => {
                    let state = self.combo_states.entry(id).or_default();
                    match key {
                        ComponentKey::Escape => state.close(),
                        ComponentKey::ArrowDown => {
                            if state.is_open() {
                                state.move_highlight(1, items.len());
                            } else {
                                state.open(items.len());
                            }
                        }
                        ComponentKey::ArrowUp => {
                            if state.is_open() {
                                state.move_highlight(-1, items.len());
                            } else {
                                state.open(items.len());
                                state.move_highlight(-1, items.len());
                            }
                        }
                        _ => {}
                    }
                    if matches!(
                        key,
                        ComponentKey::Escape | ComponentKey::ArrowDown | ComponentKey::ArrowUp
                    ) {
                        return ComponentEvents::default();
                    }
                }
                Some(ComponentKind::ListView { items, .. })
                    if key == ComponentKey::ArrowUp || key == ComponentKey::ArrowDown =>
                {
                    self.list_states.entry(id).or_default().move_selection(
                        if key == ComponentKey::ArrowUp { -1 } else { 1 },
                        items.len(),
                    );
                    return ComponentEvents::default();
                }
                Some(ComponentKind::Slider {
                    min, max, value, ..
                }) if key == ComponentKey::ArrowLeft || key == ComponentKey::ArrowRight => {
                    let next = if key == ComponentKey::ArrowLeft {
                        value.saturating_sub(1).max(min)
                    } else {
                        value.saturating_add(1).min(max)
                    };
                    self.set_slider_value(id, next);
                    return ComponentEvents::default();
                }
                Some(ComponentKind::Tabs { labels })
                    if key == ComponentKey::ArrowLeft || key == ComponentKey::ArrowRight =>
                {
                    self.tabs_states.entry(id).or_default().move_selection(
                        if key == ComponentKey::ArrowLeft {
                            -1
                        } else {
                            1
                        },
                        labels.len(),
                    );
                    return ComponentEvents::default();
                }
                _ => {}
            }
        }
        match key {
            ComponentKey::Tab => self.focus_next(bounds, shift),
            ComponentKey::Enter | ComponentKey::Space => {
                if self.focused_text_input().is_some() {
                    return ComponentEvents::default();
                }
                let Some(id) = self.focused_component() else {
                    return ComponentEvents::default();
                };
                let combo_item_count = self.find(id).and_then(|component| {
                    if let ComponentKind::ComboBox { items } = component.kind() {
                        Some(items.len())
                    } else {
                        None
                    }
                });
                if let Some(item_count) = combo_item_count {
                    let state = self.combo_states.entry(id).or_default();
                    if state.is_open() {
                        state.select_highlighted(item_count);
                    } else {
                        state.open(item_count);
                    }
                    return ComponentEvents::default();
                }
                self.activate_focused(bounds)
            }
            ComponentKey::ArrowLeft
            | ComponentKey::ArrowRight
            | ComponentKey::ArrowUp
            | ComponentKey::ArrowDown
            | ComponentKey::Escape
            | ComponentKey::WordLeft
            | ComponentKey::WordRight
            | ComponentKey::Home
            | ComponentKey::End
            | ComponentKey::Backspace
            | ComponentKey::Delete
            | ComponentKey::DeleteWordBackward
            | ComponentKey::DeleteWordForward => {
                let Some(id) = self.focused_text_input() else {
                    return ComponentEvents::default();
                };
                match key {
                    ComponentKey::ArrowLeft => {
                        self.move_text_cursor(id, false, shift);
                    }
                    ComponentKey::ArrowRight => {
                        self.move_text_cursor(id, true, shift);
                    }
                    ComponentKey::WordLeft => {
                        self.move_text_word_cursor(id, false, shift);
                    }
                    ComponentKey::WordRight => {
                        self.move_text_word_cursor(id, true, shift);
                    }
                    ComponentKey::Home => {
                        self.move_text_cursor_home_end(id, false, shift);
                    }
                    ComponentKey::End => {
                        self.move_text_cursor_home_end(id, true, shift);
                    }
                    ComponentKey::Backspace => {
                        self.backspace_text(id);
                    }
                    ComponentKey::Delete => {
                        self.delete_text(id);
                    }
                    ComponentKey::DeleteWordBackward => {
                        self.delete_word_text(id, false);
                    }
                    ComponentKey::DeleteWordForward => {
                        self.delete_word_text(id, true);
                    }
                    _ => {}
                }
                ComponentEvents::default()
            }
        }
    }

    fn sync_text_state(&mut self, id: ComponentId) -> bool {
        let Some(component) = self.find(id) else {
            return false;
        };
        let ComponentKind::TextInput { value, .. } = component.kind() else {
            return false;
        };
        let value = value.clone();
        let state = self
            .text_states
            .entry(id)
            .or_insert_with(|| TextInputState::new(value.clone()));
        if state.text() != value {
            state.set_text(value);
        }
        true
    }

    fn commit_text_state(&mut self, id: ComponentId) -> bool {
        let Some(value) = self
            .text_states
            .get(&id)
            .map(|state| state.text().to_owned())
        else {
            return false;
        };
        let Some(component) = self.find_mut(id) else {
            return false;
        };
        let ComponentKind::TextInput { value: stored, .. } = &mut component.kind else {
            return false;
        };
        *stored = value;
        true
    }

    fn dispatch_events(&mut self, events: InteractionEvents) -> ComponentEvents {
        let translated = self.translate_events(events);
        if let Some(id) = translated.activated {
            let kind = self.find(id).map(|component| component.kind().clone());
            match kind {
                Some(ComponentKind::Checkbox { .. }) => {
                    if let Some(ComponentKind::Checkbox { checked, .. }) =
                        self.find_mut(id).map(|component| &mut component.kind)
                    {
                        *checked = !*checked;
                    }
                }
                Some(ComponentKind::ComboBox { items }) => {
                    self.combo_states.entry(id).or_default().toggle(items.len());
                }
                Some(ComponentKind::Switch { .. }) => {
                    if let Some(ComponentKind::Switch { enabled, .. }) =
                        self.find_mut(id).map(|component| &mut component.kind)
                    {
                        *enabled = !*enabled;
                    }
                }
                Some(ComponentKind::RadioButton { group, .. }) => {
                    set_radio_group_selection(&mut self.roots, group, id);
                }
                _ => {}
            }
        }
        translated
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
            activated: events
                .clicked
                .and_then(|index| self.focusable_ids.get(index).copied()),
            focus_changed: events
                .focus_changed
                .and_then(|index| self.focusable_ids.get(index).copied()),
            clipboard_action: None,
        }
    }
}

fn set_radio_group_selection(nodes: &mut [UiComponent], group: u64, selected_id: ComponentId) {
    for node in nodes {
        if let ComponentKind::RadioButton {
            group: candidate_group,
            selected,
            ..
        } = &mut node.kind
        {
            if *candidate_group == group {
                *selected = node.id == selected_id;
            }
        }
        set_radio_group_selection(&mut node.children, group, selected_id);
    }
}

fn validate_unique_ids(roots: &[UiComponent]) -> Result<(), ComponentTreeError> {
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        for id in component_ids(root) {
            if !seen.insert(id) {
                return Err(ComponentTreeError::DuplicateId(id));
            }
        }
    }
    Ok(())
}

fn validate_subtree_ids(root: &UiComponent) -> Result<(), ComponentTreeError> {
    let mut seen = std::collections::HashSet::new();
    for id in component_ids(root) {
        if !seen.insert(id) {
            return Err(ComponentTreeError::DuplicateId(id));
        }
    }
    Ok(())
}

fn component_ids(root: &UiComponent) -> Vec<ComponentId> {
    fn visit(node: &UiComponent, ids: &mut Vec<ComponentId>) {
        ids.push(node.id());
        for child in node.children() {
            visit(child, ids);
        }
    }
    let mut ids = Vec::new();
    visit(root, &mut ids);
    ids
}

fn remove_component(nodes: &mut Vec<UiComponent>, id: ComponentId) -> Option<UiComponent> {
    if let Some(index) = nodes.iter().position(|node| node.id() == id) {
        return Some(nodes.remove(index));
    }
    for node in nodes {
        if let Some(removed) = remove_component(&mut node.children, id) {
            return Some(removed);
        }
    }
    None
}

fn collect_component_states(
    node: &UiComponent,
    text_states: &mut HashMap<ComponentId, TextInputState>,
    combo_states: &mut HashMap<ComponentId, ComboBoxState>,
    tabs_states: &mut HashMap<ComponentId, TabsState>,
    list_states: &mut HashMap<ComponentId, ListState>,
) {
    match node.kind() {
        ComponentKind::TextInput { value, .. } => {
            text_states.insert(node.id(), TextInputState::new(value.clone()));
        }
        ComponentKind::ComboBox { .. } => {
            combo_states.insert(node.id(), ComboBoxState::new());
        }
        ComponentKind::Tabs { .. } => {
            tabs_states.insert(node.id(), TabsState::new());
        }
        ComponentKind::ListView { .. } => {
            list_states.insert(node.id(), ListState::new());
        }
        _ => {}
    }
    for child in node.children() {
        collect_component_states(child, text_states, combo_states, tabs_states, list_states);
    }
}

fn find_component(node: &UiComponent, id: ComponentId) -> Option<&UiComponent> {
    if node.id == id {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_component(child, id))
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
    use super::{
        ComponentId, ComponentKey, ComponentKind, ComponentTree, TextInputEvent, UiComponent,
    };
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
            UiComponent::label(ComponentId(2), "Settings").with_layout_item(LayoutItem::fixed(12)),
            UiComponent::button(ComponentId(3), "Apply").with_layout_item(LayoutItem::fixed(20)),
            UiComponent::checkbox(ComponentId(4), "Enable", false)
                .with_layout_item(LayoutItem::fixed(20)),
        ]);
        ComponentTree::new(vec![panel])
    }

    #[test]
    fn checked_tree_rejects_duplicate_ids_across_descendants() {
        let duplicate = UiComponent::panel(ComponentId(90), StackLayout::default())
            .with_children(vec![UiComponent::label(ComponentId(90), "duplicate")]);
        assert!(matches!(
            ComponentTree::try_new(vec![duplicate]),
            Err(super::ComponentTreeError::DuplicateId(ComponentId(90)))
        ));
    }

    #[test]
    fn component_lifecycle_adds_and_removes_subtrees_and_text_state() {
        let mut tree = ComponentTree::try_new(vec![UiComponent::panel(
            ComponentId(91),
            StackLayout::default(),
        )])
        .unwrap();
        tree.add_child(
            ComponentId(91),
            UiComponent::text_input(ComponentId(92), "name", "hello"),
        )
        .unwrap();
        assert!(tree.text_input_state(ComponentId(92)).is_some());
        assert_eq!(
            tree.add_root(UiComponent::label(ComponentId(92), "duplicate")),
            Err(super::ComponentTreeError::DuplicateId(ComponentId(92)))
        );
        let removed = tree.remove(ComponentId(91)).unwrap();
        assert_eq!(removed.id(), ComponentId(91));
        assert!(tree.find(ComponentId(92)).is_none());
        assert!(tree.text_input_state(ComponentId(92)).is_none());
        assert!(tree.remove(ComponentId(91)).is_none());
    }

    #[test]
    fn adding_child_to_missing_parent_returns_a_typed_error() {
        let mut tree = ComponentTree::default();
        assert_eq!(
            tree.add_child(ComponentId(404), UiComponent::label(ComponentId(405), "x")),
            Err(super::ComponentTreeError::ParentNotFound(ComponentId(404)))
        );
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
        let root = UiComponent::panel(ComponentId(10), StackLayout::new(LayoutDirection::Vertical))
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
            vec!["One".into(), "Two".into(), "Three".into(), "Four".into()],
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
