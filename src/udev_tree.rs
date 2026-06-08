use super::{WhichAttributes, KEY_NAMESPACE, META_NAMESPACE, WhichProperties, XMLNameIds};
use crate::device::UdevDevice;
use frozen_collections::FzStringSet;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::mem;
use std::path::Path;
use xot::xmlname::{CreateName, CreateNamespace};
use xot::{Node, Xot};

pub struct Tree<D: UdevDevice> {
   pub document_element: Node,
   subsystems: FzStringSet<Box<str>>,
   node_map: HashMap<String, Node>,
   fragment_map: HashMap<String, Node>,
   which_attributes: WhichAttributes,
   which_properties: WhichProperties,
   pub names: XMLNameIds,
   _phantom: PhantomData<D>,
}

impl<D: UdevDevice> Tree<D> {
   pub fn new(
      xot: &mut Xot,
      which_attributes: WhichAttributes,
      which_properties: WhichProperties,
      subsystems: FzStringSet<Box<str>>,
   ) -> Self {
      // create an empty root document
      let document_element = {
         let name = xot.add_name("udev");
         xot.new_element(name)
      };
      let document_node = xot
         .new_document_with_element(document_element)
         .unwrap();

      // TODO eventually remove
      assert_eq!(xot.document_element(document_node).unwrap(), document_element);

      let meta_ns = CreateNamespace::new(xot, META_NAMESPACE.0, META_NAMESPACE.1);
      xot.append_namespace(document_element, &meta_ns).unwrap();

      let managed = CreateName::namespaced(xot, "managed", &meta_ns).name_id();

      let key_ns = CreateNamespace::new(xot, KEY_NAMESPACE.0, KEY_NAMESPACE.1);
      xot.append_namespace(document_element, &key_ns).unwrap();

      let device = xot.add_name("device");
      let attr = xot.add_name("attr");
      let attr_path = xot.add_name("attr-path");
      let name = xot.add_name("name");
      let value = xot.add_name("value");

      let sysname = CreateName::namespaced(xot, "SYSNAME", &key_ns).name_id();
      let syspath = CreateName::namespaced(xot, "SYSPATH", &key_ns).name_id();
      let subsystem = CreateName::namespaced(xot, "SUBSYSTEM", &key_ns).name_id();
      let driver = CreateName::namespaced(xot, "DRIVER", &key_ns).name_id();

      Self {
         document_element,
         subsystems,
         node_map: HashMap::new(),
         fragment_map: HashMap::new(),
         which_attributes,
         which_properties,
         names: XMLNameIds {
            device,
            attr,
            attr_path,
            name,
            value,
            sysname,
            syspath,
            subsystem,
            driver,
            managed,
         },
         _phantom: PhantomData,
      }
   }

   fn set_properties(&mut self, xot: &mut Xot, node: Node, dev: &D, is_managed: bool) {
      // removing all old properties
      xot.attributes_mut(node).clear();

      let keys = [(self.names.syspath, dev.syspath()), (self.names.sysname, dev.sysname())]
         .into_iter()
         .chain(dev.subsystem().map(|s| (self.names.subsystem, s)))
         .chain(dev.driver().map(|s| (self.names.driver, s)))
         .chain(is_managed.then(|| (self.names.managed, "")));

      for (id, value) in keys {
         // .attributes_mut() is necessary and cheap
         xot.attributes_mut(node).insert(id, value.to_owned());
      }

      if matches!(self.which_properties, WhichProperties::All) {
         dev.for_each_property(|name, value| {
            // .attributes_mut() is necessary and cheap
            let id = xot.add_name(name);
            xot.attributes_mut(node).insert(id, value.to_owned());
         });
      }
   }

   fn set_attributes(&mut self, xot: &mut Xot, node: Node, dev: &D) {
      if matches!(self.which_attributes, WhichAttributes::None) {
         return;
      }

      // removing all old attributes
      let mut cur_child = xot.first_child(node);
      while let Some(child_node) = cur_child {
         let next = xot.next_sibling(child_node);
         if xot
            .element(child_node)
            .is_some_and(|e| e.name() == self.names.attr || e.name() == self.names.attr_path)
         {
            xot.remove(child_node).unwrap()
         }
         cur_child = next;
      }

      let mut prefixes = HashMap::new();
      prefixes.insert(Path::new("").to_owned(), node);
      dev.for_each_attribute(|name, value| {
         if matches!(self.which_attributes, WhichAttributes::Tree | WhichAttributes::Both) {
            let path = Path::new(&name);
            let mut ancestors = path.ancestors();
            let mut last_node = None;
            let mut found = false;

            while !found && let Some(prefix) = ancestors.next() {
               let (node, mut attrs) = if let Some(&node) = prefixes.get(prefix) {
                  found = true;
                  (node, xot.attributes_mut(node))
               } else {
                  let element = prefix.file_name().unwrap();
                  let node = xot.new_element(self.names.attr);
                  let mut attrs = xot.attributes_mut(node);
                  attrs.insert(self.names.name, element.to_str().unwrap().to_owned());
                  prefixes.insert(prefix.to_owned(), node);
                  (node, attrs)
               };

               if let Some(last) = last_node {
                  xot.append(node, last).unwrap();
               } else {
                  attrs.insert(self.names.value, value.to_owned());
               }
               last_node = Some(node);
            }
         }
         if matches!(self.which_attributes, WhichAttributes::ByPath | WhichAttributes::Both) {
            let attr_path = xot.new_element(self.names.attr_path);
            let mut attrs = xot.attributes_mut(attr_path);
            attrs.insert(self.names.name, name.to_owned());
            attrs.insert(self.names.value, value.to_owned());
            xot.append(node, attr_path).unwrap();
         }
      });
   }

   fn get_fragment_parent(&mut self, xot: &mut Xot, key: &str) -> Node {
      if let Some(parent) = self.fragment_map.get(key) {
         *parent
      } else {
         let parent = xot.new_element(self.names.device);
         self.fragment_map.insert(key.to_owned(), parent);
         parent
      }
   }

   fn take_or_create(&mut self, xot: &mut Xot, key: &str) -> Node {
      self
         .fragment_map
         .remove(key)
         .map_or_else(|| xot.new_element(self.names.device), |parent| parent)
   }

   fn is_managed(&self, subsystem: Option<&str>) -> bool {
      self.subsystems.is_empty() || subsystem.is_some_and(|subsystem| self.subsystems.contains(subsystem))
   }

   pub fn attach(&mut self, xot: &mut Xot, device: D) {
      if let Some(&node) = self.node_map.get(device.syspath()) {
         // update attributes if the device is already in the tree
         self.set_properties(xot, node, &device, true);
         self.set_attributes(xot, node, &device);
         return;
      }

      // create and append ancestors until we reach the tree
      let mut tree_cur = self.document_element;
      let mut dev_cur = device;
      let mut node_cur = self.take_or_create(xot, dev_cur.syspath());
      let mut first = true;

      while let Some(dev_parent) = {
         self.node_map.insert(dev_cur.syspath().to_owned(), node_cur);
         self.set_properties(xot, node_cur, &dev_cur, mem::take(&mut first));
         self.set_attributes(xot, node_cur, &dev_cur);

         dev_cur.parent().and_then(|dev_parent| {
            if let Some(&node) = self.node_map.get(dev_parent.syspath()) {
               tree_cur = node;
               None
            } else {
               if self.is_managed(dev_parent.subsystem()) {
                  tree_cur = self.get_fragment_parent(xot, dev_cur.syspath());
                  None
               } else {
                  Some(dev_parent)
               }
            }
         })
      } {
         let parent = xot.new_element(self.names.device);
         xot.append(parent, node_cur).unwrap();
         node_cur = parent;
         dev_cur = dev_parent;
      }
      xot.append(tree_cur, node_cur).unwrap();
   }

   pub fn detach(&mut self, xot: &mut Xot, device: D) {
      // if an ancestor came from a different subsystem, it might have already removed
      // this node
      if let Some(mut node) = self.node_map.remove(device.syspath())
         && !xot.is_removed(node)
      {
         while let Some(parent) = {
            xot.parent(node).filter(|parent| {
               !xot.attributes(*parent).contains_key(self.names.managed) && xot.children(*parent).one_or_less_items()
            })
         } {
            node = parent;
         }
         xot.remove(node).unwrap();
      }
   }
}

trait OneItemed: Iterator {
   #[allow(unused)]
   fn exactly_one_item(self) -> bool;
   fn one_or_less_items(self) -> bool;
}

impl<T> OneItemed for T
where
   T: Iterator,
{
   fn exactly_one_item(mut self) -> bool {
      self.next().is_some() && self.next().is_none()
   }

   fn one_or_less_items(mut self) -> bool {
      self.next();
      self.next().is_none()
   }
}
