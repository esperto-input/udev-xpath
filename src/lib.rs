use crate::device::UdevDevice;
use crate::udev_tree::{KEY_NAMESPACE, META_NAMESPACE, UdevTree};
use futures::future::LocalBoxFuture;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use indexmap::IndexMap;
use indexmap::map::Entry;
use std::io::{Error, ErrorKind};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, RwLockWriteGuard};
use tokio::time::sleep;
use tokio_udev::AsyncMonitorSocket;
use udev::{Device, Enumerator};

use udev::{EventType as UdevEventType, MonitorBuilder};
use xee_xpath::context::StaticContextBuilder;
use xee_xpath::{Documents, Queries};
use xot::output::Indentation;
use xot::output::xml::Parameters;
use xot::{Node, Xot};

mod device;
mod udev_tree;

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

pub struct AsyncTree {
   docs: Arc<RwLock<Documents>>,
   tasks: FuturesUnordered<LocalBoxFuture<'static, Option<Result<(Event, AsyncMonitorSocket), Error>>>>,
   events: IndexMap<Box<str>, Event>,
   tree: UdevTree<Device>,
   timeout: Duration,
}

impl AsyncTree {
   pub async fn new() -> Result<AsyncTree, Box<dyn std::error::Error>> {
      Self::with_duration(Duration::from_secs(1)).await
   }

   pub async fn with_duration(timeout: Duration) -> Result<AsyncTree, Box<dyn std::error::Error>> {
      let mut docs = Documents::new();
      let xot = docs.xot_mut();
      let mut tree = UdevTree::new_raw(xot);

      let mut connected_devices = Enumerator::new()?;
      connected_devices.match_subsystem("input")?;

      for device in connected_devices.scan_devices()? {
         tree.attach(xot, device);
      }

      let builder = MonitorBuilder::new()?.match_subsystem("input")?;
      let monitor = builder.listen()?;
      let hotplug_devices = AsyncMonitorSocket::new(monitor)?;

      let tasks = FuturesUnordered::from_iter([Self::next_event(hotplug_devices).boxed_local()]);

      Ok(AsyncTree {
         docs: Arc::new(RwLock::new(docs)),
         tasks,
         events: IndexMap::with_capacity(5),
         tree,
         timeout,
      })
   }

   pub async fn ready(&mut self) -> Result<(), Error> {
      // tasks always contains at most one next_event and one timer
      //    next_event yields Some(result)
      //    the timer yields None
      // tasks starts with exactly one next_event
      // in case of error tasks starts either empty or with the leftover timer
      // in either case we abort
      while {
         let (event, hotplug_devices) = self
            .tasks
            .next()
            .await
            .flatten()
            .unwrap_or(Err(Error::from(ErrorKind::BrokenPipe)))?; // no tasks or leftover timer after previous failure

         self.events.insert(UdevDevice::syspath(&event.device).into(), event);

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
         while let Some((new, hotplug_devices)) = self.tasks.next().await.unwrap().transpose()? {
            let syspath = UdevDevice::syspath(&new.device);
            match self.events.entry(syspath.to_owned().into_boxed_str()) {
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
                        let (key, _) = entry.shift_remove_entry();
                        self.events.insert(key, new);
                     }
                     (Kind::Remove, Kind::Add) => {
                        *old = Event {
                           kind: Kind::Update,
                           device: new.device,
                        };
                     }
                     _ => {
                        return Err(Error::from(ErrorKind::Unsupported));
                     }
                  }
               }
               Entry::Vacant(entry) => {
                  entry.insert(new);
               }
            }
            self.tasks.push(Self::next_event(hotplug_devices).boxed_local());
         }

         if self.events.is_empty() {
            true
         } else {
            let mut docs = self.docs.write().await;
            let xot = docs.xot_mut();
            for (_, Event { kind: tpe, device }) in self.events.drain(..) {
               match tpe {
                  Kind::Add | Kind::Update => {
                     self.tree.attach(xot, device);
                  }
                  Kind::Remove => {
                     self.tree.detach(xot, device);
                  }
               }
            }
            false
         }
      } {}
      Ok(())
   }

   async fn next_event(mut hotplug_devices: AsyncMonitorSocket) -> Option<Result<(Event, AsyncMonitorSocket), Error>> {
      Some(loop {
         break match hotplug_devices
            .next()
            .await
            .unwrap_or(Err(Error::from(ErrorKind::BrokenPipe)))
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

   pub fn get_handle(self) -> DocumentsHandle {
      DocumentsHandle {
         docs: Arc::clone(&self.docs),
         document_element: self.tree.document_element,
         _phantom: Default::default(),
      }
   }

   pub async fn to_string(&self) -> Result<String, xot::Error> {
      let docs = self.docs.read().await;
      let xot = docs.xot();

      let mut config = Parameters::default();
      config.indentation = Some(Indentation::default());
      xot.serialize_xml_string(config, self.tree.document_element)
   }
}

pub struct DocumentsHandle {
   docs: Arc<RwLock<Documents>>,
   document_element: Node,
   _phantom: PhantomData<Arc<RwLock<Xot>>>, // this is where I would put my xot, if I had one!
}

impl DocumentsHandle {
   pub async fn get(&self) -> RwLockWriteGuard<'_, Documents> {
      self.docs.write().await
   }

   pub fn static_context() -> StaticContextBuilder<'static> {
      let mut static_context_builder = StaticContextBuilder::default();
      static_context_builder.add_namespace(META_NAMESPACE.0, META_NAMESPACE.1);
      static_context_builder.add_namespace(KEY_NAMESPACE.0, KEY_NAMESPACE.1);
      static_context_builder
   }

   pub fn queries() -> Queries<'static> {
      Queries::new(Self::static_context())
   }

   pub async fn to_string(&self) -> Result<String, xot::Error> {
      let docs = self.docs.read().await;
      let xot = docs.xot();

      let mut config = Parameters::default();
      config.indentation = Some(Indentation::default());
      xot.serialize_xml_string(config, self.document_element)
   }
}
