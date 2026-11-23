//! Applies item modifiers (vanilla `LootItemFunction`s), as used by `/item modify`
//! and data pack `item_modifier` files.
//!
//! A modifier is a function object, a list of them (an inline sequence), or the id of
//! a data pack item modifier. Modifiers are read as NBT so command SNBT and data pack
//! JSON share one path.

use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::read_data;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::random::{RandomImpl, get_seed, xoroshiro128::Xoroshiro};
use tracing::debug;

/// Item modifiers that reference each other deeper than this are treated as a cycle.
const MAX_REFERENCE_DEPTH: usize = 64;

/// Applies `modifier` to `stack` and returns the result. `lookup` resolves the ids of
/// data pack item modifiers. Functions that aren't supported leave the stack as is.
#[must_use]
pub fn apply_item_modifier(
    stack: ItemStack,
    modifier: &NbtTag,
    lookup: &dyn Fn(&str) -> Option<NbtTag>,
) -> ItemStack {
    Applier {
        lookup,
        rng: Xoroshiro::from_seed(get_seed()),
    }
    .apply(stack, modifier, 0)
}

struct Applier<'a> {
    lookup: &'a dyn Fn(&str) -> Option<NbtTag>,
    rng: Xoroshiro,
}

impl Applier<'_> {
    fn apply(&mut self, stack: ItemStack, modifier: &NbtTag, depth: usize) -> ItemStack {
        if depth > MAX_REFERENCE_DEPTH {
            debug!("Item modifier nesting is too deep, skipping the rest");
            return stack;
        }
        match modifier {
            NbtTag::String(id) => self.apply_reference(stack, id, depth),
            NbtTag::List(functions) => functions.iter().fold(stack, |stack, function| {
                self.apply(stack, function, depth + 1)
            }),
            NbtTag::Compound(function) => self.apply_function(stack, function, depth),
            _ => stack,
        }
    }

    fn apply_reference(&mut self, stack: ItemStack, id: &str, depth: usize) -> ItemStack {
        let Some(modifier) = (self.lookup)(id) else {
            debug!("Unknown item modifier {id}");
            return stack;
        };
        self.apply(stack, &modifier, depth + 1)
    }

    fn apply_function(
        &mut self,
        mut stack: ItemStack,
        function: &NbtCompound,
        depth: usize,
    ) -> ItemStack {
        // 26.x dispatches on `type`; older packs used `function`.
        let kind = function
            .get_string("type")
            .or_else(|| function.get_string("function"))
            .unwrap_or_default();
        let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
        if function
            .get("conditions")
            .and_then(NbtTag::extract_list)
            .is_some_and(|conditions| !conditions.is_empty())
        {
            debug!("Item modifier function {kind} has conditions, which aren't supported yet");
            return stack;
        }
        match kind {
            "sequence" => match function.get("functions") {
                Some(functions) => self.apply(stack, functions, depth + 1),
                None => stack,
            },
            "reference" => match function.get_string("name") {
                Some(id) => self.apply_reference(stack, id, depth),
                None => stack,
            },
            "set_components" => {
                if let Some(components) = function.get_compound("components") {
                    set_components(&mut stack, components);
                }
                stack
            }
            "set_item" => {
                let Some(id) = function.get_string("item") else {
                    return stack;
                };
                let Some(item) =
                    Item::from_registry_key(id.strip_prefix("minecraft:").unwrap_or(id))
                else {
                    debug!("Unknown item {id} in set_item");
                    return stack;
                };
                stack.transmute_copy(item, stack.item_count)
            }
            "set_count" => {
                let Some(count) = self.number(function.get("count")) else {
                    return stack;
                };
                let base = if function.get_bool("add").unwrap_or(false) {
                    i32::from(stack.item_count)
                } else {
                    0
                };
                // `item_count` can't go above `u8::MAX`; callers limit it to the stack size.
                stack.item_count = (base + count as i32).clamp(0, i32::from(u8::MAX)) as u8;
                stack
            }
            "set_damage" => {
                let Some(max_damage) = stack.get_max_damage().filter(|max| *max > 0) else {
                    return stack;
                };
                let Some(damage) = self.number(function.get("damage")) else {
                    return stack;
                };
                let base = if function.get_bool("add").unwrap_or(false) {
                    1.0 - stack.get_damage() as f32 / max_damage as f32
                } else {
                    0.0
                };
                let missing = 1.0 - (damage + base).clamp(0.0, 1.0);
                stack.set_damage((missing * max_damage as f32).floor() as i32);
                stack
            }
            other => {
                debug!("Unsupported item modifier function {other}");
                stack
            }
        }
    }

    /// Reads a constant or uniform number provider.
    fn number(&mut self, provider: Option<&NbtTag>) -> Option<f32> {
        let provider = provider?;
        if let Some(value) = provider.as_numeric_float() {
            return Some(value);
        }
        let provider = provider.extract_compound()?;
        let kind = provider.get_string("type").unwrap_or("minecraft:uniform");
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "constant" => provider.get("value")?.as_numeric_float(),
            "uniform" => {
                let min = provider.get("min")?.as_numeric_float()?;
                let max = provider.get("max")?.as_numeric_float()?;
                Some(if min >= max {
                    min
                } else {
                    min + self.rng.next_f32() * (max - min)
                })
            }
            other => {
                debug!("Unsupported number provider {other} in an item modifier");
                None
            }
        }
    }
}

/// Applies a component patch like vanilla `ItemStack.applyComponents`: `!name` removes a
/// component and every other entry replaces it. Components Pumpkin can't read are skipped.
fn set_components(stack: &mut ItemStack, components: &NbtCompound) {
    for (name, value) in &components.child_tags {
        let (removal, name) = name
            .strip_prefix('!')
            .map_or((false, &**name), |name| (true, name));
        let Some(id) = DataComponent::try_from_name(name) else {
            debug!("Unknown component {name} in set_components");
            continue;
        };
        let value = if removal {
            None
        } else {
            let Some(value) = read_data(id, value) else {
                debug!("Couldn't read component {name} in set_components");
                continue;
            };
            Some(value)
        };
        stack.patch.retain(|(existing, _)| *existing != id);
        let is_default = match (&value, stack.item.components.iter().find(|(c, _)| *c == id)) {
            (Some(value), Some((_, default))) => value.equal(*default),
            (None, None) => true,
            _ => false,
        };
        if !is_default {
            stack.patch.push((id, value));
        }
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_data::data_component::DataComponent;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;
    use pumpkin_nbt::tag::NbtTag;

    use super::apply_item_modifier;
    use crate::data::datapack::dynamic_registry_loader::json_to_nbt_tag;

    fn modifier(json: &str) -> NbtTag {
        json_to_nbt_tag(&serde_json::from_str(json).expect("valid json"))
    }

    fn no_lookup(_: &str) -> Option<NbtTag> {
        None
    }

    #[test]
    fn set_components_adds_and_removes_components() {
        let stack = ItemStack::new(1, &Item::APPLE);
        let stack = apply_item_modifier(
            stack,
            &modifier(
                r#"{
                    "type": "minecraft:set_components",
                    "components": {
                        "minecraft:item_model": "matcha:amber",
                        "minecraft:custom_data": {"version": 1},
                        "!minecraft:food": {}
                    }
                }"#,
            ),
            &no_lookup,
        );
        assert!(stack.has_data_component(DataComponent::ItemModel));
        assert!(stack.has_data_component(DataComponent::CustomData));
        assert!(!stack.has_data_component(DataComponent::Food));
        assert_eq!(stack.patch.len(), 3);
    }

    #[test]
    fn named_modifiers_and_sequences_apply_in_order() {
        let lookup = |id: &str| {
            (id == "matcha:amber").then(|| {
                modifier(
                    r#"{
                        "type": "minecraft:sequence",
                        "functions": [
                            {"type": "minecraft:set_item", "item": "minecraft:poisonous_potato"},
                            {"type": "set_components", "components": {"item_model": "matcha:amber"}}
                        ]
                    }"#,
                )
            })
        };
        let stack = ItemStack::new(3, &Item::APPLE);
        let stack = apply_item_modifier(stack, &NbtTag::String("matcha:amber".into()), &lookup);
        assert_eq!(stack.item.id, Item::POISONOUS_POTATO.id);
        assert_eq!(stack.item_count, 3);
        assert!(stack.has_data_component(DataComponent::ItemModel));
    }

    #[test]
    fn set_count_and_set_damage_match_vanilla() {
        let stack = ItemStack::new(2, &Item::APPLE);
        let stack = apply_item_modifier(
            stack,
            &modifier(r#"[{"function": "set_count", "count": 3, "add": true}]"#),
            &no_lookup,
        );
        assert_eq!(stack.item_count, 5);

        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        let max = sword.get_max_damage().expect("sword has durability");
        let sword = apply_item_modifier(
            sword,
            &modifier(r#"{"type": "set_damage", "damage": 0.25}"#),
            &no_lookup,
        );
        assert_eq!(sword.get_damage(), (0.75 * max as f32).floor() as i32);
    }

    #[test]
    fn reference_cycles_stop() {
        let lookup = |_: &str| Some(NbtTag::String("test:loop".into()));
        let stack = apply_item_modifier(
            ItemStack::new(1, &Item::APPLE),
            &NbtTag::String("test:loop".into()),
            &lookup,
        );
        assert_eq!(stack.item_count, 1);
    }
}
