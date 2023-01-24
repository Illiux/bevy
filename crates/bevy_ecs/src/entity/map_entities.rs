use crate::{entity::Entity, world::World};
use bevy_utils::{Entry, HashMap};
use std::fmt;

/// The errors that might be returned while using [`MapEntities::map_entities`].
#[derive(Debug)]
pub enum MapEntitiesError {
    EntityNotFound(Entity),
}

impl std::error::Error for MapEntitiesError {}

impl fmt::Display for MapEntitiesError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MapEntitiesError::EntityNotFound(_) => {
                write!(f, "the given entity does not exist in the map")
            }
        }
    }
}

/// Operation to map all contained [`Entity`] fields in a type to new values.
///
/// As entity IDs are valid only for the [`World`] they're sourced from, using [`Entity`]
/// as references in components copied from another world will be invalid. This trait
/// allows defining custom mappings for these references via [`EntityMap`].
///
/// Implementing this trait correctly is required for properly loading components
/// with entity references from scenes.
///
/// ## Example
///
/// ```rust
/// use bevy_ecs::prelude::*;
/// use bevy_ecs::entity::{EntityMapper, MapEntities, MapEntitiesError};
///
/// #[derive(Component)]
/// struct Spring {
///     a: Entity,
///     b: Entity,
/// }
///
/// impl MapEntities for Spring {
///     fn map_entities(&mut self, entity_map: &mut EntityMapper) -> Result<(), MapEntitiesError> {
///         self.a = entity_map.get(self.a)?;
///         self.b = entity_map.get(self.b)?;
///         Ok(())
///     }
/// }
/// ```
///
/// [`World`]: crate::world::World
pub trait MapEntities {
    /// Updates all [`Entity`] references stored inside using `entity_map`.
    ///
    /// Implementors should look up any and all [`Entity`] values stored within and
    /// update them to the mapped values via `entity_map`.
    fn map_entities(&mut self, entity_mapper: &mut EntityMapper) -> Result<(), MapEntitiesError>;
}

/// A mapping from one set of entities to another.
///
/// The API generally follows [`HashMap`], but each [`Entity`] is returned by value, as they are [`Copy`].
///
/// This is typically used to coordinate data transfer between sets of entities, such as between a scene and the world or over the network.
/// This is required as [`Entity`] identifiers are opaque; you cannot and do not want to reuse identifiers directly.
#[derive(Default, Debug)]
pub struct EntityMap {
    map: HashMap<Entity, Entity>,
}

/// A wrapper for [`EntityMap`], augmenting it with the ability to allocate new [`Entity`] references in a destination
/// world. These newly allocated references are guaranteed to never point to any living entity in that world.
///
/// References are allocated by returning increasing generations starting from an internally initialized base
/// [`Entity`]. After it is finished being used by [`MapEntities`] implementations, this entity is despawned and the
/// requisite number of generations reserved.
pub struct EntityMapper<'m> {
    /// The wrapped [`EntityMap`].
    map: &'m mut EntityMap,
    /// A base [`Entity`] used to allocate new references.
    dead_start: Entity,
    /// The number of generations this mapper has allocated thus far.
    generations: u32,
}

impl<'m> EntityMapper<'m> {
    /// Returns the corresponding mapped entity.
    pub fn get(&self, entity: Entity) -> Result<Entity, MapEntitiesError> {
        self.map.get(entity)
    }

    /// Returns the corresponding mapped entity or allocates a new dead entity if it is absent.
    pub fn get_or_alloc(&mut self, entity: Entity) -> Entity {
        if let Ok(mapped) = self.map.get(entity) {
            return mapped;
        }

        let new = Entity {
            generation: self.dead_start.generation + self.generations,
            index: self.dead_start.index,
        };
        self.generations += 1;

        self.map.insert(entity, new);

        new
    }

    /// Gets a reference to the underlying [`EntityMap`].
    pub fn get_map(&'m self) -> &'m EntityMap {
        self.map
    }

    /// Gets a mutable reference to the underlying [`EntityMap`]
    pub fn get_map_mut(&'m mut self) -> &'m mut EntityMap {
        self.map
    }

    /// Creates a new [`EntityMapper`], spawning a temporary base [`Entity`] in the provided [`World`]
    fn new(map: &'m mut EntityMap, world: &mut World) -> Self {
        Self {
            map,
            dead_start: world.spawn_empty().id(),
            generations: 0,
        }
    }

    /// Reserves the allocated references to dead entities within the world. This despawns the temporary base
    /// [`Entity`] while reserving extra generations via [`World::try_reserve_generations`]. Because this renders the
    /// [`EntityMapper`] unable to safely allocate any more references, this method takes ownership of `self` in order
    /// to render it unusable.
    fn save(self, world: &mut World) {
        if self.generations == 0 {
            assert!(world.despawn(self.dead_start));
            return;
        }

        assert!(world.try_reserve_generations(self.dead_start, self.generations));
    }
}

impl EntityMap {
    /// Inserts an entities pair into the map.
    ///
    /// If the map did not have `from` present, [`None`] is returned.
    ///
    /// If the map did have `from` present, the value is updated, and the old value is returned.
    pub fn insert(&mut self, from: Entity, to: Entity) -> Option<Entity> {
        self.map.insert(from, to)
    }

    /// Removes an `entity` from the map, returning the mapped value of it if the `entity` was previously in the map.
    pub fn remove(&mut self, entity: Entity) -> Option<Entity> {
        self.map.remove(&entity)
    }

    /// Gets the given entity's corresponding entry in the map for in-place manipulation.
    pub fn entry(&mut self, entity: Entity) -> Entry<'_, Entity, Entity> {
        self.map.entry(entity)
    }

    /// Returns the corresponding mapped entity.
    pub fn get(&self, entity: Entity) -> Result<Entity, MapEntitiesError> {
        self.map
            .get(&entity)
            .cloned()
            .ok_or(MapEntitiesError::EntityNotFound(entity))
    }

    /// An iterator visiting all keys in arbitrary order.
    pub fn keys(&self) -> impl Iterator<Item = Entity> + '_ {
        self.map.keys().cloned()
    }

    /// An iterator visiting all values in arbitrary order.
    pub fn values(&self) -> impl Iterator<Item = Entity> + '_ {
        self.map.values().cloned()
    }

    /// Returns the number of elements in the map.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns true if the map contains no elements.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// An iterator visiting all (key, value) pairs in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, Entity)> + '_ {
        self.map.iter().map(|(from, to)| (*from, *to))
    }

    /// Calls the provided closure with an [`EntityMapper`] created from this [`EntityMap`]. This allows the closure
    /// to allocate new entity references in the provided [`World`] that will never point at a living entity.
    pub fn with_mapper<R>(
        &mut self,
        world: &mut World,
        f: impl FnOnce(&mut World, &mut EntityMapper) -> R,
    ) -> R {
        let mut mapper = EntityMapper::new(self, world);
        let result = f(world, &mut mapper);
        mapper.save(world);
        result
    }
}
