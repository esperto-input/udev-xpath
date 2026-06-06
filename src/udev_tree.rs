use crate::device::UdevDevice;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::Path;
use xot::xmlname::{CreateName, CreateNamespace};
use xot::{NameId, Node, Xot};

pub const META_NAMESPACE: (&str, &str) = ("_", "https://mirolang.org/meta_ns");
pub const KEY_NAMESPACE: (&str, &str) = ("key", "https://mirolang.org/key_ns");

struct XMLNames {
   device: NameId,
   attr: NameId,
   attr_path: NameId,
   name: NameId,
   value: NameId,
   sysname: NameId,
   syspath: NameId,
   subsystem: NameId,
   driver: NameId,
   input: NameId,
}

pub struct UdevTree<D: UdevDevice> {
   pub document_element: Node,
   node_map: HashMap<String, Node>,
   names: XMLNames,
   _phantom: PhantomData<D>,
}

impl<D: UdevDevice> UdevTree<D> {
   pub fn new_raw(xot: &mut Xot) -> Self {
      // create an empty root document
      let document_element = {
         let name = xot.add_name("udev");
         xot.new_element(name)
      };
      let document_node = xot
         .new_document_with_element(document_element)
         .expect("Failed to create base XML document");

      // TODO eventually remove
      assert_eq!(xot.document_element(document_node).unwrap(), document_element);

      let meta_ns = CreateNamespace::new(xot, META_NAMESPACE.0, META_NAMESPACE.1);
      xot.append_namespace(document_element, &meta_ns).unwrap();

      let input = CreateName::namespaced(xot, "input", &meta_ns).name_id();

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
         node_map: HashMap::new(),
         names: XMLNames {
            device,
            attr,
            attr_path,
            name,
            value,
            sysname,
            syspath,
            subsystem,
            driver,
            input,
         },
         _phantom: PhantomData,
      }
   }

   pub fn attach(&mut self, xot: &mut Xot, device: D) {
      if let Some(&node) = self.node_map.get(device.syspath()) {
         // update attributes if the device is already in the tree
         self.set_properties(xot, node, &device);
         self.set_attributes(xot, node, &device);
         return;
      }

      // create and append ancestors until we reach the tree
      let mut tree_cur = self.document_element;
      let mut dev_cur = device;
      let mut node_cur = xot.new_element(self.names.device);

      while let Some(dev_parent) = {
         self.node_map.insert(dev_cur.syspath().to_owned(), node_cur);
         self.set_properties(xot, node_cur, &dev_cur);
         self.set_attributes(xot, node_cur, &dev_cur);

         dev_cur.parent().and_then(|dev_parent| {
            self
               .node_map
               .get(dev_parent.syspath())
               .map_or(Some(dev_parent), |&node| {
                  tree_cur = node;
                  None
               })
         })
      } {
         let parent = xot.new_element(self.names.device);
         xot.append(parent, node_cur).unwrap();
         node_cur = parent;
         dev_cur = dev_parent;
      }
      xot.append(tree_cur, node_cur).unwrap();
   }

   fn set_properties(&mut self, xot: &mut Xot, node: Node, dev: &D) {
      // removing all old properties
      xot.attributes_mut(node).clear();

      let subsystem = dev.subsystem();
      let keys = [(self.names.syspath, dev.syspath()), (self.names.sysname, dev.sysname())]
         .into_iter()
         .chain(subsystem.map(|s| (self.names.subsystem, s)))
         .chain(dev.driver().map(|s| (self.names.driver, s)))
         .chain(subsystem.is_some_and(|s| s == "input").then(|| (self.names.input, "")));

      for (id, value) in keys {
         // .xot_mut() and .attributes_mut() are both necessary and cheap
         xot.attributes_mut(node).insert(id, value.to_owned());
      }

      dev.for_each_property(|name, value| {
         // .xot_mut() and .attributes_mut() are both necessary and cheap
         let id = xot.add_name(name);
         xot.attributes_mut(node).insert(id, value.to_owned());
      });
   }

   fn set_attributes(&mut self, xot: &mut Xot, node: Node, dev: &D) {
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
      for (name, value) in dev.attributes() {
         // attr as tree
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
               attrs.insert(self.names.value, value.clone());
            }
            last_node = Some(node);
         }

         // attr by path
         let attr_path = xot.new_element(self.names.attr_path);
         let mut attrs = xot.attributes_mut(attr_path);
         attrs.insert(self.names.name, name);
         attrs.insert(self.names.value, value);
         xot.append(node, attr_path).unwrap();
      }
   }

   pub fn detach(&mut self, xot: &mut Xot, device: D) {
      if let Some(mut node) = self.node_map.remove(device.syspath()) {
         while let Some(parent) = {
            xot.parent(node).filter(|parent| {
               !xot.attributes(*parent).contains_key(self.names.input) && xot.children(*parent).one_or_less_items()
            })
         } {
            node = parent;
         }
         xot.remove(node).unwrap();
      }
   }
}

trait OneItemed: Iterator {
   // fn exactly_one_item(self) -> bool;
   fn one_or_less_items(self) -> bool;
}

impl<T> OneItemed for T
where
   T: Iterator,
{
   // fn exactly_one_item(mut self) -> bool {
   //    self.next().is_some() && self.next().is_none()
   // }

   fn one_or_less_items(mut self) -> bool {
      self.next();
      self.next().is_none()
   }
}
