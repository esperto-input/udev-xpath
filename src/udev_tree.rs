use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::marker::PhantomData;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use tap::Pipe;
use udev::Device;
use xee_xpath::context::StaticContextBuilder;
pub use xee_xpath::error::Result as XeeResult;
use xee_xpath::query::{Convert, ManyQuery, OneQuery};
use xee_xpath::{Documents, Item, Queries, Query};
use xot::output::xml::Parameters;
use xot::output::Indentation;
use xot::xmlname::{CreateName, CreateNamespace};
use xot::{Error, NameId, Node};

const META_NAMESPACE: (&str, &str) = ("_", "https://mirolang.org/meta_ns");
const KEY_NAMESPACE: (&str, &str) = ("key", "https://mirolang.org/key_ns");

pub trait UdevDevice: Sized {
   fn syspath(&self) -> &str;

   fn parent(&self) -> Option<Self>;

   fn sysname(&self) -> &str;

   fn subsystem(&self) -> Option<&str>;

   fn driver(&self) -> Option<&str>;

   fn attributes(&self) -> impl Iterator<Item = (String, String)>;

   fn properties(&self) -> impl Iterator<Item = (String, String)>;

   fn for_each_attribute(&self, callback: impl FnMut(&str, &str));

   fn for_each_property(&self, callback: impl FnMut(&str, &str));
}

impl UdevDevice for Device {
   fn syspath(&self) -> &str {
      self.syspath().to_str().unwrap()
   }

   fn parent(&self) -> Option<Self> {
      self.parent()
   }

   fn sysname(&self) -> &str {
      self.sysname().to_str().unwrap()
   }

   fn subsystem(&self) -> Option<&str> {
      self.subsystem().map(OsStr::to_str).flatten()
   }

   fn driver(&self) -> Option<&str> {
      self.driver().map(OsStr::to_str).flatten()
   }

   fn attributes(&self) -> impl Iterator<Item = (String, String)> {
      self.attributes().map(|entry| {
         (
            entry.name().to_str().unwrap().to_owned(),
            entry.value().pipe(|value| {
               value
                  .to_str()
                  .map_or_else(|| STANDARD.encode(value.as_bytes()), str::to_owned)
            }),
         )
      })
   }

   fn properties(&self) -> impl Iterator<Item = (String, String)> {
      self.properties().map(|entry| {
         (
            entry.name().to_str().unwrap().to_owned(),
            entry.value().pipe(|value| {
               value
                  .to_str()
                  .map_or_else(|| STANDARD.encode(value.as_bytes()), str::to_owned)
            }),
         )
      })
   }

   fn for_each_attribute(&self, mut callback: impl FnMut(&str, &str)) {
      self.attributes().for_each(|entry| {
         let name = entry.name().to_str().unwrap();
         let value = entry.value();
         if let Some(value) = value.to_str() {
            callback(name, value);
         } else {
            callback(name, &STANDARD.encode(value.as_bytes()));
         }
      })
   }

   fn for_each_property(&self, mut callback: impl FnMut(&str, &str)) {
      self.properties().for_each(|entry| {
         let name = entry.name().to_str().unwrap();
         let value = entry.value();
         if let Some(value) = value.to_str() {
            callback(name, value);
         } else {
            callback(name, &STANDARD.encode(value.as_bytes()));
         }
      })
   }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct DummyDevice {
   pub syspath: String,
   pub sysname: String,
   pub driver: Option<String>,
   pub subsystem: Option<String>,
   pub properties: HashMap<String, String>,
   pub attributes: HashMap<String, String>,
   pub parent: Option<Box<DummyDevice>>,
}

#[cfg(test)]
impl UdevDevice for DummyDevice {
   fn syspath(&self) -> &str {
      self.syspath.as_str()
   }

   fn parent(&self) -> Option<Self> {
      self.parent.as_ref().map(|parent| *parent.clone())
   }

   fn sysname(&self) -> &str {
      self.sysname.as_str()
   }

   fn subsystem(&self) -> Option<&str> {
      self.subsystem.as_ref().map(String::as_str)
   }

   fn driver(&self) -> Option<&str> {
      self.driver.as_ref().map(String::as_str)
   }

   fn attributes(&self) -> impl Iterator<Item = (String, String)> {
      self.attributes.iter().map(|(a, b)| (a.clone(), b.clone()))
   }

   fn properties(&self) -> impl Iterator<Item = (String, String)> {
      self.properties.iter().map(|(a, b)| (a.clone(), b.clone()))
   }

   fn for_each_attribute(&self, mut callback: impl FnMut(&str, &str)) {
      self.attributes.iter().for_each(|entry| callback(entry.0, entry.1));
   }

   fn for_each_property(&self, mut callback: impl FnMut(&str, &str)) {
      self.properties.iter().for_each(|entry| callback(entry.0, entry.1));
   }
}

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
   docs: Documents,
   document_element: Node,
   node_map: HashMap<String, Node>,
   names: XMLNames,
   static_context_builder: StaticContextBuilder<'static>,
   _phantom: PhantomData<D>,
}

impl<D: UdevDevice> UdevTree<D> {
   pub fn new() -> Self {
      let mut docs = Documents::new();
      let xot = docs.xot_mut();

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
      // let single_child = CreateName::namespaced(xot, "input", &meta_ns).name_id();

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

      let mut static_context_builder = StaticContextBuilder::default();
      static_context_builder.add_namespace(META_NAMESPACE.0, META_NAMESPACE.1);
      static_context_builder.add_namespace(KEY_NAMESPACE.0, KEY_NAMESPACE.1);

      Self {
         docs,
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
         static_context_builder,
         _phantom: PhantomData,
      }
   }

   pub fn attach(&mut self, device: D) {
      // declr_syspath_ref!(syspath, device);
      if let Some(&node) = self.node_map.get(device.syspath()) {
         // update attributes if the device is already in the tree
         self.set_properties(node, &device);
         self.set_attributes(node, &device);
         return;
      }

      // create and append ancestors until we reach the tree
      let mut tree_cur = self.document_element;
      let mut dev_cur = device;
      let mut node_cur = self.docs.xot_mut().new_element(self.names.device);

      while let Some(dev_parent) = {
         self.node_map.insert(dev_cur.syspath().to_owned(), node_cur);
         self.set_properties(node_cur, &dev_cur);
         self.set_attributes(node_cur, &dev_cur);

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
         let xot = self.docs.xot_mut();
         let parent = xot.new_element(self.names.device);
         xot.append(parent, node_cur).unwrap();
         node_cur = parent;
         dev_cur = dev_parent;
      }
      self.docs.xot_mut().append(tree_cur, node_cur).unwrap();
   }

   fn set_properties(&mut self, node: Node, dev: &D) {
      // removing all old properties
      self.docs.xot_mut().attributes_mut(node).clear();

      let subsystem = dev.subsystem();
      let keys = [(self.names.syspath, dev.syspath()), (self.names.sysname, dev.sysname())]
         .into_iter()
         .chain(subsystem.map(|s| (self.names.subsystem, s)))
         .chain(dev.driver().map(|s| (self.names.driver, s)))
         .chain(subsystem.is_some_and(|s| s == "input").then(|| (self.names.input, "")));

      for (id, value) in keys {
         // .xot_mut() and .attributes_mut() are both necessary and cheap
         let xot = self.docs.xot_mut();
         xot.attributes_mut(node).insert(id, value.to_owned());
      }

      dev.for_each_property(|name, value| {
         // .xot_mut() and .attributes_mut() are both necessary and cheap
         let xot = self.docs.xot_mut();
         let id = xot.add_name(name);
         xot.attributes_mut(node).insert(id, value.to_owned());
      });
   }

   fn set_attributes(&mut self, node: Node, dev: &D) {
      // removing all old attributes
      let xot = self.docs.xot_mut();
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

   pub fn detach(&mut self, syspath: &str) {
      if let Some(mut node) = self.node_map.remove(syspath) {
         let xot = self.docs.xot_mut();
         while let Some(parent) = {
            xot.parent(node).filter(|parent| {
               xot.attributes(*parent).contains_key(self.names.input) && !xot.children(*parent).one_or_less_items()
            })
         } {
            node = parent;
         }

         xot.remove(node).unwrap();
      }
   }

   pub fn compile_boolean_query(&self, query_str: &str) -> XeeResult<OneQuery<bool, impl Convert<bool> + use<D>>> {
      let queries = Queries::new(self.static_context_builder.clone());
      queries.one(query_str, move |_, item| Ok(item.try_into_value::<bool>()?))
   }

   pub fn test_boolean_query(&mut self, syspath: &str, query: &OneQuery<bool, impl Convert<bool>>) -> XeeResult<bool> {
      let node = match self.node_map.get(syspath) {
         Some(&node) => node,
         None => return Ok(false),
      };
      query.execute(&mut self.docs, node)
   }

   pub fn compile_and_test_boolean_query(&mut self, syspath: &str, query_str: &str) -> XeeResult<bool> {
      let query = self.compile_boolean_query(query_str)?;
      self.test_boolean_query(syspath, &query)
   }

   pub fn match_query(&mut self, query: &ManyQuery<String, impl Convert<String>>) -> XeeResult<Vec<String>> {
      query.execute(&mut self.docs, self.document_element)
   }

   pub fn compile_query(&self, query_str: &str) -> XeeResult<ManyQuery<String, impl Convert<String> + use<D>>> {
      let name = self.names.syspath;
      Queries::new(self.static_context_builder.clone()).many(
         query_str,
         move |documents: &mut Documents, item: &Item| {
            let xot = documents.xot_mut();
            let attrs = xot.attributes(item.to_node()?);
            Ok(attrs.get(name).unwrap().clone())
         },
      )
   }

   pub fn compile_and_match(&mut self, query_str: &str) -> XeeResult<Vec<String>> {
      let query = self.compile_query(query_str)?;
      query.execute(&mut self.docs, self.document_element)
   }

   pub fn serialize(&self) -> Result<String, Error> {
      let xot = self.docs.xot();

      let mut config = Parameters::default();
      config.indentation = Some(Indentation::default());
      xot.serialize_xml_string(config, self.document_element)
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
