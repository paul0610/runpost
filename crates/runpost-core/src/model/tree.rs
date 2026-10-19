//! Operaciones sobre el árbol de una colección (API del sprint 3: crear, editar, mover,
//! duplicar y borrar carpetas y peticiones).

use super::{Collection, Item, new_id};
use crate::error::CoreError;

fn find<'a>(items: &'a [Item], id: &str) -> Option<&'a Item> {
    for item in items {
        if item.id() == id {
            return Some(item);
        }
        if let Item::Folder(folder) = item
            && let Some(found) = find(&folder.items, id)
        {
            return Some(found);
        }
    }
    None
}

fn find_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Item> {
    for item in items.iter_mut() {
        if item.id() == id {
            return Some(item);
        }
        if let Item::Folder(folder) = item
            && let Some(found) = find_mut(&mut folder.items, id)
        {
            return Some(found);
        }
    }
    None
}

fn remove(items: &mut Vec<Item>, id: &str) -> Option<Item> {
    if let Some(index) = items.iter().position(|item| item.id() == id) {
        return Some(items.remove(index));
    }
    items.iter_mut().find_map(|item| match item {
        Item::Folder(folder) => remove(&mut folder.items, id),
        Item::Request(_) => None,
    })
}

/// Dónde insertar dentro de la lista de destino.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Position {
    /// Carpeta destino; `None` = raíz de la colección.
    pub parent: Option<String>,
    /// Insertar antes de este hermano (si existe en el destino).
    pub before: Option<String>,
    /// O en este índice (se recorta al tamaño de la lista).
    pub index: Option<usize>,
}

impl Item {
    /// Ids nuevos para el elemento y todo lo que contiene (duplicar, importar copias).
    pub fn fresh_ids(&mut self) {
        match self {
            Item::Folder(folder) => {
                folder.id = new_id("fld");
                folder.items.iter_mut().for_each(Item::fresh_ids);
            }
            Item::Request(request) => request.id = new_id("req"),
        }
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        match self {
            Item::Folder(folder) => folder.name = name.into(),
            Item::Request(request) => request.name = name.into(),
        }
    }

    fn contains(&self, id: &str) -> bool {
        match self {
            Item::Folder(folder) => folder.items.iter().any(|i| i.id() == id || i.contains(id)),
            Item::Request(_) => false,
        }
    }
}

impl Collection {
    pub fn find_item(&self, id: &str) -> Option<&Item> {
        find(&self.items, id)
    }

    pub fn find_item_mut(&mut self, id: &str) -> Option<&mut Item> {
        find_mut(&mut self.items, id)
    }

    /// Hijos de `parent` (`None` = raíz). Error si no existe o no es una carpeta.
    fn children_mut(&mut self, parent: Option<&str>) -> Result<&mut Vec<Item>, CoreError> {
        match parent {
            None => Ok(&mut self.items),
            Some(id) => match self.find_item_mut(id) {
                Some(Item::Folder(folder)) => Ok(&mut folder.items),
                Some(Item::Request(_)) => {
                    Err(CoreError::InvalidRequest(format!("{id} is not a folder")))
                }
                None => Err(CoreError::not_found("item", id)),
            },
        }
    }

    /// Inserta `item` en `position`. Si su id está vacío o ya existe, se le dan ids nuevos.
    pub fn insert_item(
        &mut self,
        position: &Position,
        mut item: Item,
    ) -> Result<String, CoreError> {
        if item.id().is_empty() || self.find_item(item.id()).is_some() {
            item.fresh_ids();
        }
        let id = item.id().to_owned();
        let list = self.children_mut(position.parent.as_deref())?;
        let at = position
            .before
            .as_deref()
            .and_then(|before| list.iter().position(|i| i.id() == before))
            .or(position.index.map(|index| index.min(list.len())))
            .unwrap_or(list.len());
        list.insert(at, item);
        Ok(id)
    }

    pub fn remove_item(&mut self, id: &str) -> Option<Item> {
        remove(&mut self.items, id)
    }

    /// Carpeta padre (`None` = raíz) y posición del elemento `id` entre sus hermanos.
    pub fn locate(&self, id: &str) -> Option<(Option<String>, usize)> {
        fn walk(items: &[Item], id: &str, parent: Option<&str>) -> Option<(Option<String>, usize)> {
            for (index, item) in items.iter().enumerate() {
                if item.id() == id {
                    return Some((parent.map(str::to_owned), index));
                }
                if let Item::Folder(folder) = item
                    && let Some(found) = walk(&folder.items, id, Some(&folder.id))
                {
                    return Some(found);
                }
            }
            None
        }
        walk(&self.items, id, None)
    }

    /// Mueve un elemento dentro de la colección. No permite meter una carpeta en sí misma.
    pub fn move_item(&mut self, id: &str, position: &Position) -> Result<(), CoreError> {
        let item = self
            .find_item(id)
            .ok_or_else(|| CoreError::not_found("item", id))?;
        if let Some(parent) = &position.parent
            && (parent == id || item.contains(parent))
        {
            return Err(CoreError::InvalidRequest(
                "cannot move a folder into itself".into(),
            ));
        }
        // Validar el destino antes de quitar nada.
        self.children_mut(position.parent.as_deref())?;
        let item = self
            .remove_item(id)
            .ok_or_else(|| CoreError::not_found("item", id))?;
        let list = self.children_mut(position.parent.as_deref())?;
        let at = position
            .before
            .as_deref()
            .and_then(|before| list.iter().position(|i| i.id() == before))
            .or(position.index.map(|index| index.min(list.len())))
            .unwrap_or(list.len());
        list.insert(at, item);
        Ok(())
    }

    /// Copia el elemento justo detrás del original, con ids nuevos y el nombre dado.
    pub fn duplicate_item(&mut self, id: &str, name: Option<&str>) -> Result<String, CoreError> {
        let mut copy = self
            .find_item(id)
            .cloned()
            .ok_or_else(|| CoreError::not_found("item", id))?;
        copy.fresh_ids();
        let new_name = name
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} copy", copy.name()));
        copy.set_name(new_name);
        let new_id = copy.id().to_owned();
        let parent = self.parent_of(id);
        let list = self.children_mut(parent.as_deref())?;
        let at = list
            .iter()
            .position(|i| i.id() == id)
            .map_or(list.len(), |i| i + 1);
        list.insert(at, copy);
        Ok(new_id)
    }

    /// Id de la carpeta que contiene `id` (`None` si está en la raíz o no existe).
    pub fn parent_of(&self, id: &str) -> Option<String> {
        fn walk(items: &[Item], id: &str, parent: Option<&str>) -> Option<Option<String>> {
            for item in items {
                if item.id() == id {
                    return Some(parent.map(str::to_owned));
                }
                if let Item::Folder(folder) = item
                    && let Some(found) = walk(&folder.items, id, Some(&folder.id))
                {
                    return Some(found);
                }
            }
            None
        }
        walk(&self.items, id, None).flatten()
    }

    /// Ids nuevos para la colección y todo su árbol (duplicar una colección).
    pub fn fresh_ids(&mut self) {
        self.id = new_id("col");
        self.items.iter_mut().for_each(Item::fresh_ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Folder, Request};

    fn tree() -> Collection {
        let mut c = Collection::new("C");
        let mut a = Folder::new("A");
        a.id = "fld_a".into();
        let mut b = Folder::new("B");
        b.id = "fld_b".into();
        let mut r1 = Request::new("r1", "GET", "http://x/1");
        r1.id = "req_1".into();
        let mut r2 = Request::new("r2", "GET", "http://x/2");
        r2.id = "req_2".into();
        b.items.push(Item::Request(Box::new(r2)));
        a.items.push(Item::Request(Box::new(r1)));
        a.items.push(Item::Folder(b));
        c.items.push(Item::Folder(a));
        c
    }

    fn names(items: &[Item]) -> Vec<&str> {
        items.iter().map(Item::name).collect()
    }

    #[test]
    fn find_and_parent() {
        let mut c = tree();
        assert_eq!(c.find_item("req_2").unwrap().name(), "r2");
        assert_eq!(c.parent_of("req_2").as_deref(), Some("fld_b"));
        assert_eq!(c.parent_of("fld_a"), None);
        assert_eq!(c.parent_of("nope"), None);
        c.find_item_mut("req_1").unwrap().set_name("renamed");
        assert_eq!(c.find_item("req_1").unwrap().name(), "renamed");
    }

    #[test]
    fn insert_at_root_folder_before_and_index() {
        let mut c = tree();
        let new = Item::Request(Box::new(Request::new("n", "GET", "http://n")));
        let id = c.insert_item(&Position::default(), new.clone()).unwrap();
        assert_eq!(c.items.last().unwrap().id(), id);
        let in_folder = c
            .insert_item(
                &Position {
                    parent: Some("fld_a".into()),
                    before: Some("fld_b".into()),
                    index: None,
                },
                new.clone(),
            )
            .unwrap();
        let Item::Folder(a) = &c.items[0] else {
            panic!()
        };
        assert_eq!(a.items[1].id(), in_folder);
        // Un id repetido se renueva para no duplicar ids en el árbol.
        let mut dup = new.clone();
        if let Item::Request(r) = &mut dup {
            r.id = "req_1".into();
        }
        let fresh = c
            .insert_item(
                &Position {
                    parent: None,
                    before: None,
                    index: Some(0),
                },
                dup,
            )
            .unwrap();
        assert_ne!(fresh, "req_1");
        assert_eq!(c.items[0].id(), fresh);
        let error = c.insert_item(
            &Position {
                parent: Some("req_1".into()),
                ..Position::default()
            },
            new.clone(),
        );
        assert_eq!(error.unwrap_err().code(), "invalid_request");
        let missing = c.insert_item(
            &Position {
                parent: Some("fld_x".into()),
                ..Position::default()
            },
            new,
        );
        assert_eq!(missing.unwrap_err().code(), "item_not_found");
    }

    #[test]
    fn move_and_guard_against_cycles() {
        let mut c = tree();
        c.move_item(
            "req_2",
            &Position {
                parent: None,
                before: Some("fld_a".into()),
                index: None,
            },
        )
        .unwrap();
        assert_eq!(names(&c.items), ["r2", "A"]);
        c.move_item(
            "req_2",
            &Position {
                parent: Some("fld_a".into()),
                ..Position::default()
            },
        )
        .unwrap();
        let Item::Folder(a) = &c.items[0] else {
            panic!()
        };
        assert_eq!(names(&a.items), ["r1", "B", "r2"]);
        assert!(
            c.move_item(
                "fld_a",
                &Position {
                    parent: Some("fld_b".into()),
                    ..Position::default()
                }
            )
            .is_err()
        );
        assert!(
            c.move_item(
                "fld_a",
                &Position {
                    parent: Some("fld_a".into()),
                    ..Position::default()
                }
            )
            .is_err()
        );
        assert!(
            c.move_item(
                "req_1",
                &Position {
                    parent: Some("req_2".into()),
                    ..Position::default()
                }
            )
            .is_err()
        );
        assert!(c.find_item("req_1").is_some(), "failed move keeps the item");
        assert_eq!(
            c.move_item("nope", &Position::default())
                .unwrap_err()
                .code(),
            "item_not_found"
        );
    }

    #[test]
    fn duplicate_remove_and_fresh_ids() {
        let mut c = tree();
        let copy = c.duplicate_item("fld_b", Some("B (copia)")).unwrap();
        let Item::Folder(a) = &c.items[0] else {
            panic!()
        };
        assert_eq!(names(&a.items), ["r1", "B", "B (copia)"]);
        let Item::Folder(b2) = &a.items[2] else {
            panic!()
        };
        assert_eq!(b2.id, copy);
        assert_ne!(b2.items[0].id(), "req_2");
        assert_eq!(c.duplicate_item("req_1", None).map(|_| ()), Ok(()));
        let Item::Folder(a) = &c.items[0] else {
            panic!()
        };
        assert_eq!(a.items[1].name(), "r1 copy");
        assert_eq!(c.remove_item("req_2").unwrap().name(), "r2");
        assert!(c.remove_item("req_2").is_none());
        let old = c.id.clone();
        c.fresh_ids();
        assert_ne!(c.id, old);
        assert!(c.find_item("fld_a").is_none());
    }
}
