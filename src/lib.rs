#![doc = include_str!("../README.md")]

use frozen_collections::FzStringSet;
use futures::future::LocalBoxFuture;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use indexmap::IndexMap;
use indexmap::map::Entry;
use std::error::Error;
use std::io::ErrorKind;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;
use std::{io, iter};
use tap::Tap;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use tokio::time::sleep;
use tokio_udev::AsyncMonitorSocket;
use udev::{Device, Enumerator};
use udev::{EventType as UdevEventType, MonitorBuilder};
use xee_xpath::context::StaticContextBuilder;
use xee_xpath::{Documents, Queries};
use xot::output::Indentation;
use xot::output::xml::Parameters;
use xot::{NameId, Node, Xot};

use device::UdevDevice;
use udev_tree::Tree;

mod device;
mod udev_tree;

macro_rules! dprintln {
    ($($arg:tt)*) => {
       #[cfg(debug_assertions)]
       ::std::println!($($arg)*)
    };
}

const WINDOW_WARMUP: usize = 5;

/// Label and value of the `key` namespace
pub const KEY_NAMESPACE: (&str, &str) = ("key", "https://mirolang.org/key_ns");

/// Label and value of the `_` (meta) namespace
pub const META_NAMESPACE: (&str, &str) = ("_", "https://mirolang.org/meta_ns");

/// Name IDs of common XML strings
#[derive(Copy, Clone)]
pub struct XMLNameIds {
   /// `"device"` string, used in `<device/>` tags.
   ///
   /// `device` tags represent udev device nodes. Udev **keys** are represented
   /// as XML attributes with namespace `key`, udev device **properties** are
   /// represented as XML attributes with no namespace. Device attributes are
   /// offered both as `attr` and `attr-path` tags.
   ///
   /// ```xml
   /// <udev> <!-- root -->
   ///   <device key:KEY="value" PROPERTY="value"> <!-- parent device -->
   ///     <device/> <!-- child device -->
   ///   </device>
   /// </udev>
   pub device: NameId,
   /// `"attr"` string, used in `<attr/>` tags.
   ///
   /// `attr` tags represent device **attributes** in a nested manner.
   ///
   /// ```xml
   /// <device>
   ///   <attr name="capabilities">
   ///     <attr name="abs" value="0"/>
   ///     <attr name="ev" value="21"/>
   ///   </attr>
   /// </device>
   /// ```
   pub attr: NameId,
   /// `"attr-path"` string, used in `<attr-path/>` tags.
   ///
   /// `attr-path` tags represent device **attributes** in a flattened manner,
   /// with their full path. This allows for simpler queries in some situations.
   ///
   /// ```xml
   /// <device
   ///   <attr-path name="capabilities/abs" value="0"/>
   ///   <attr-path name="capabilities/ev" value="21"/>
   /// </device
   /// ```
   pub attr_path: NameId,
   /// `"name"` string, used in `name="attribute_name"` XML attributes.
   ///
   /// Name attributes hold the name of a device attribute
   pub name: NameId,
   /// `"value"` string, used in `value="attribute_value"` XML attributes.
   ///
   /// Value attributes hold the value of a device attribute
   pub value: NameId,
   /// `"key:SYSNAME"` string, used in `key:SYSNAME="sysname"` XML attributes.
   ///
   /// Holds the `SYSNAME` udev device key
   pub sysname: NameId,
   /// `"key:SYSPATH"` string, used in `key:SYSPATH="syspath"` XML attributes.
   ///
   /// Holds the `SYSPATH` udev device key
   pub syspath: NameId,
   /// `"key:SUBSYSTEM"` string, used in `key:SUBSYSTEM="subsystem"` XML
   /// attributes.
   ///
   /// Holds the optional `SUBSYSTEM` udev device key
   pub subsystem: NameId,
   /// `"key:DRIVER"` string, used in `key:DRIVER="driver"` XML attributes.
   ///
   /// Holds the optional `DRIVER` udev device key
   pub driver: NameId,
   /// `"_:managed"` string, used in `_:managed=""` XML attributes.
   ///
   /// Intended for internal use, this attribute holds no value. This attribute
   /// is **set** on devices that are received from the udev monitor and
   /// **unset** on implicit devices that are required to build a tree path.
   /// When listening on all subsystems, the attribute will be set on all
   /// devices. Otherwise, only to those from specified subsystems.
   pub managed: NameId,
}

/// Which attributes to generate in the tree
#[derive(Copy, Clone)]
pub enum WhichAttributes {
   /// Only generate the tree-like `attr` tags
   ///
   /// ```xml
   /// <device>
   ///   <attr name="capabilities">
   ///     <attr name="abs" value="0"/>
   ///     <attr name="ev" value="21"/>
   ///   </attr>
   /// </device>
   /// ```
   Tree,

   /// Only generate the full-path `attr-path` tags
   ///
   /// ```xml
   /// <device
   ///   <attr-path name="capabilities/abs" value="0"/>
   ///   <attr-path name="capabilities/ev" value="21"/>
   /// </device
   /// ```
   ByPath,

   /// Generate both type of udev attribute tags
   Both,

   /// Dont generate attribute tags
   None,
}

/// Which properties to generate in the tree
#[derive(Copy, Clone)]
pub enum WhichProperties {
   /// Generate all udev properties (as XML attributes)
   ///
   /// ```xml
   /// <device key:KEY="value" PROPERTY="value"/>
   /// ```
   All,

   /// Don't generate udev properties. Udev keys will still be available.
   ///
   /// ```xml
   /// <device key:KEY="value"/>
   None,
}

/// Builder for [`UdevTree`]
pub struct UdevTreeBuilder<R: AsRef<str>, I: IntoIterator<Item = R>> {
   timeout: Duration,
   attributes: WhichAttributes,
   properties: WhichProperties,
   subsystems: I,
}

impl UdevTreeBuilder<&'static str, iter::Empty<&'static str>> {
   fn default() -> Self {
      UdevTreeBuilder {
         timeout: Duration::from_secs(2),
         attributes: WhichAttributes::Both,
         properties: WhichProperties::All,
         subsystems: iter::empty(),
      }
   }
}

impl<R: AsRef<str>, I: IntoIterator<Item = R>> UdevTreeBuilder<R, I> {
   /// Time window for coalescing calls to [`UdevTree::ready`]
   pub fn timeout(mut self, timeout: Duration) -> Self {
      self.timeout = timeout;
      self
   }

   /// Which attributes to include in the tree
   pub fn attributes(mut self, attributes: WhichAttributes) -> Self {
      self.attributes = attributes;
      self
   }

   /// Which properties to include in the tree
   pub fn properties(mut self, properties: WhichProperties) -> Self {
      self.properties = properties;
      self
   }

   /// Which subsystems to listen on. If subsystems are unspecified or empty,
   /// listens to all subsystems. The last call replaces the previous.
   pub fn subsystems<R2: AsRef<str>, I2: IntoIterator<Item = R2>>(self, subsystems: I2) -> UdevTreeBuilder<R2, I2> {
      UdevTreeBuilder {
         timeout: self.timeout,
         attributes: self.attributes,
         properties: self.properties,
         subsystems,
      }
   }

   /// Builds the tree
   pub async fn build(self) -> Result<UdevTree, Box<dyn Error>> {
      UdevTree::new(self).await
   }
}

/// Maintains an in-memory XML representation of the udev device database.
/// To keep the tree updated [`UdevTree::ready`] is to be repeatedly awaited on.
///
/// To access and query the device tree use the [`DocumentsContext`] object
/// returned by [`UdevTree::get_documents_context`].
pub struct UdevTree {
   docs: Arc<RwLock<Documents>>,
   tasks: FuturesUnordered<LocalBoxFuture<'static, Option<Result<(Event, AsyncMonitorSocket), io::Error>>>>,
   window: IndexMap<Box<str>, Event>,
   tree: Tree<Device>,
   timeout: Duration,
   broken: bool,
}

impl UdevTree {
   pub fn builder() -> UdevTreeBuilder<&'static str, iter::Empty<&'static str>> {
      UdevTreeBuilder::default()
   }

   async fn new<R: AsRef<str>, I: IntoIterator<Item = R>>(
      UdevTreeBuilder {
         timeout,
         attributes: which_attributes,
         properties: which_properties,
         subsystems,
      }: UdevTreeBuilder<R, I>,
   ) -> Result<Self, Box<dyn Error>> {
      let subsystems = FzStringSet::from_iter(subsystems);
      let tasks = if subsystems.is_empty() {
         let builder = MonitorBuilder::new()?;
         let monitor = builder.listen()?;
         let hotplug_devices = AsyncMonitorSocket::new(monitor)?;
         FuturesUnordered::from_iter([Self::next_event(hotplug_devices).boxed_local()])
      } else {
         let tasks = FuturesUnordered::new();
         for subsystem in subsystems.iter() {
            let builder = MonitorBuilder::new()?.match_subsystem(subsystem.as_ref())?;
            let monitor = builder.listen()?;
            let hotplug_devices = AsyncMonitorSocket::new(monitor)?;
            tasks.push(Self::next_event(hotplug_devices).boxed_local());
         }
         tasks
      };

      let enumerators = if subsystems.is_empty() {
         [Enumerator::new()?].into()
      } else {
         subsystems
            .iter()
            .map(|subsystem| {
               let mut connected_devices = Enumerator::new()?;
               connected_devices.match_subsystem(subsystem.as_ref())?;
               Ok(connected_devices)
            })
            .collect::<Result<Box<[_]>, io::Error>>()?
      };

      let mut docs = Documents::new();

      let xot = docs.xot_mut();
      let mut tree = Tree::new(xot, which_attributes, which_properties, subsystems);
      for mut enumerator in enumerators {
         for device in enumerator.scan_devices()? {
            tree.attach(xot, device);
         }
      }

      Ok(UdevTree {
         docs: Arc::new(RwLock::new(docs)),
         tasks,
         window: IndexMap::with_capacity(WINDOW_WARMUP),
         tree,
         timeout,
         broken: false,
      })
   }

   /// This method **must** be called repeatedly to keep the device tree
   /// updated. The method blocks until the device tree has changed, and
   /// returns at any change. Note that the updated nodes might be temporarily
   /// unreachable from the document root.
   /// ([`DocumentsContext::document_element`]).
   ///
   /// Bursts of device changes in a time window of [`UdevTreeBuilder::timeout`]
   /// are coalesced/merged into a single call. This behaviour can be
   /// employed to throttle the re-running of queries or other. Note that if a
   /// device is connected and disconnected within the time window, it will
   /// not be added to the tree and the method will not return.
   ///
   /// After the first error, subsequent calls will return immediately with an
   /// [`ErrorKind::BrokenPipe`] error, without updating the tree. A new tree
   /// should be created from scratch, and document context redistributed.
   pub async fn ready(&mut self) -> Result<(), io::Error> {
      if self.broken {
         return Err(io::Error::from(ErrorKind::BrokenPipe));
      }
      'a: {
         macro_rules! yeet {
            ($expression:expr) => {
               match $expression {
                  Ok(val) => val,
                  Err(err) => break 'a Err(err),
               }
            };
         }
         while {
            // - next_event yields Some(result)
            // - the timer yields None
            // tasks starts with at least one next_event
            // errors are handled above, timer is handled below
            let (event, hotplug_devices) = yeet!(self.tasks.next().await.flatten().unwrap());

            self.window.insert(UdevDevice::syspath(&event.device).into(), event);

            self.tasks.push(Self::next_event(hotplug_devices).boxed_local());
            let timeout = self.timeout;
            self.tasks.push(
               async move {
                  sleep(timeout).await;
                  None
               }
               .boxed_local(),
            );

            // at this point tasks cannot be empty, we can unwrap the first option
            // Option<Result<T, E>> transposed to Result<Option<T>, E>
            while let Some((new, hotplug_devices)) = yeet!(self.tasks.next().await.unwrap().transpose()) {
               let syspath = UdevDevice::syspath(&new.device);
               match self.window.entry(syspath.to_owned().into_boxed_str()) {
                  Entry::Occupied(mut entry) => {
                     let old = entry.get_mut();
                     match (old.kind, new.kind) {
                        (Kind::Add, Kind::Update) => {
                           old.device = new.device;
                        }
                        (Kind::Add, Kind::Remove) => {
                           entry.shift_remove();
                        }
                        (Kind::Update, Kind::Update) => {
                           old.device = new.device;
                        }
                        (Kind::Update, Kind::Remove) => {
                           // TODO comment on this
                           let (key, _) = entry.shift_remove_entry();
                           self.window.insert(key, new);
                        }
                        (Kind::Remove, Kind::Add) => {
                           *old = Event {
                              kind: Kind::Update,
                              device: new.device,
                           };
                        }
                        _ => {
                           return Err(io::Error::from(ErrorKind::Unsupported));
                        }
                     }
                  }
                  Entry::Vacant(entry) => {
                     entry.insert(new);
                  }
               }
               self.tasks.push(Self::next_event(hotplug_devices).boxed_local());
            }

            if self.window.is_empty() {
               dprintln!("Nothing to do");
               true
            } else {
               let mut docs = self.docs.write().await;
               let xot = docs.xot_mut();
               for (syspath, Event { kind: tpe, device }) in self.window.drain(..) {
                  match tpe {
                     Kind::Add | Kind::Update => {
                        dprintln!("adding/updating {syspath}");
                        self.tree.attach(xot, device);
                     }
                     Kind::Remove => {
                        dprintln!("removing {syspath}");
                        self.tree.detach(xot, device);
                     }
                  }
               }
               false
            }
         } {}
         Ok(())
      }
      .tap(|result| self.broken = result.is_err())
   }

   async fn next_event(
      mut hotplug_devices: AsyncMonitorSocket,
   ) -> Option<Result<(Event, AsyncMonitorSocket), io::Error>> {
      Some(loop {
         break match hotplug_devices
            .next()
            .await
            .unwrap_or(Err(io::Error::from(ErrorKind::BrokenPipe)))
         {
            Ok(event) => {
               let event_type = event.event_type();
               let device = event.device();

               Ok((
                  match event_type {
                     UdevEventType::Add => Event {
                        kind: Kind::Add,
                        device,
                     },
                     UdevEventType::Change | UdevEventType::Bind | UdevEventType::Unbind => Event {
                        kind: Kind::Update,
                        device,
                     },
                     UdevEventType::Remove => Event {
                        kind: Kind::Remove,
                        device,
                     },
                     UdevEventType::Unknown => continue,
                  },
                  hotplug_devices,
               ))
            }
            Err(e) => Err(e),
         };
      })
   }

   /// Returns a new instance of [`DocumentsContext`]. More instances can also
   /// be cloned from existing ones.
   pub fn get_documents_context(&self) -> DocumentsContext {
      DocumentsContext {
         docs: Arc::clone(&self.docs),
         document_element: self.tree.document_element,
         names: self.tree.names,
         _phantom: Default::default(),
      }
   }

   /// Serialize the device tree as an XML string
   pub async fn to_string(&self) -> Result<String, xot::Error> {
      let docs = self.docs.read().await;
      let xot = docs.xot();

      let mut config = Parameters::default();
      config.indentation = Some(Indentation::default());
      xot.serialize_xml_string(config, self.tree.document_element)
   }
}

/// This object is used to get a safe handle to a [`Documents`] using the
/// [`DocumentsContext::get`] method, used for querying the tree.
/// The handle is meant to be thrown away every time, while this context is
/// meant to be reused to get a new handle.
///
/// Internally, this uses an [`RwLock`], so [`UdevTree::ready`] will block
/// until the handle is dropped.
pub struct DocumentsContext {
   docs: Arc<RwLock<Documents>>,
   document_element: Node,
   names: XMLNameIds,
   _phantom: PhantomData<Arc<RwLock<Xot>>>, // this is where I would put my xot, if I had one!
}

impl DocumentsContext {
   /// Get a handle to [`Documents`]. The handle holds a
   /// [`RwLockWriteGuard`] on the device tree.
   ///
   /// Currently, due to limitations in [`xee_xpath`], it's not possible to run
   /// queries in a readonly tree, so the method locks with write access
   /// instead. This behaviour has a performance penalty (queries cannot run in
   /// parallel) and will change when read-only queries are available.
   /// In the future, write access to the tree will not be obtainable.
   ///
   /// If you only need read access, for the time being you can use
   /// [`DocumentsContext::get_read`]
   ///
   /// > [!WARNING]
   /// > **Do not** rely on having write access to the documents, as it will not
   /// > be possible in the future. **Do not** modify the tree, as it will most
   /// > likely break the library algorithms.
   pub async fn get(&self) -> RwLockWriteGuard<'_, Documents> {
      self.docs.write().await
   }

   /// Read-only version of [`DocumentsContext::get`]. This will be removed in
   /// the future.
   pub async fn get_read(&self) -> RwLockReadGuard<'_, Documents> {
      self.docs.read().await
   }

   /// Query static context, with the `key` and `_` namespaces
   pub fn static_context() -> StaticContextBuilder<'static> {
      let mut static_context_builder = StaticContextBuilder::default();
      static_context_builder.add_namespace(META_NAMESPACE.0, META_NAMESPACE.1);
      static_context_builder.add_namespace(KEY_NAMESPACE.0, KEY_NAMESPACE.1);
      static_context_builder
   }

   /// Returns a query collection with the correct namespaces for running
   /// queries on the documents. This is the recommended way of building
   /// queries.
   pub fn queries() -> Queries<'static> {
      Queries::new(Self::static_context())
   }

   /// Collection of the known NameIDs, check [`XMLNameIds`] for details. Useful
   /// when running queries, or exploring the device tree.
   ///
   /// If you need a different NameId, or can't use this collection, you can get
   /// names directly using [`Xot::name`] and other methods.
   pub fn name_ids(&self) -> &XMLNameIds {
      &self.names
   }

   /// Root of the device tree. Most queries may need to start from this node.
   pub fn document_element(&self) -> &Node {
      &self.document_element
   }

   /// Serialize the device tree as an XML string
   pub async fn to_string(&self) -> Result<String, xot::Error> {
      let docs = self.docs.read().await;
      let xot = docs.xot();

      let mut config = Parameters::default();
      config.indentation = Some(Indentation::default());
      xot.serialize_xml_string(config, self.document_element)
   }
}

impl Clone for DocumentsContext {
   fn clone(&self) -> Self {
      Self {
         docs: Arc::clone(&self.docs),
         document_element: self.document_element,
         names: self.names,
         _phantom: PhantomData,
      }
   }
}

#[derive(Copy, Clone)]
enum Kind {
   Add,
   Update,
   Remove,
}

struct Event {
   kind: Kind,
   device: Device,
}
