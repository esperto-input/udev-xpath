use tokio::select;
use tokio::task::LocalSet;
use udev_xpath::{DocumentsContext, UdevTree};
use xee_xpath::{Documents, Item, Query};

/// The [`UdevTree::ready`] future cannot be [`Send`], so if you intend
/// to use spawn it as a task, you must use [`tokio::task::spawn_local`].
/// That requires either using `tokio::main(flavor = "current_thread")` or a
/// [`LocalSet`]. This example uses a local set, which can only be awaited from
/// main.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
   let mut tree = UdevTree::builder().subsystems(["input", "hid"]).build().await?;

   let handle1 = tree.get_documents_context();
   let handle2 = handle1.clone();
   let syspath = handle1.name_ids().syspath;
   let query1 = DocumentsContext::queries().many(
      "//*[@SUBSYSTEM='hid']/*/*/self::device[@ID_INPUT='1' and matches(@key:SYSNAME, 'event\\d+')]",
      move |documents: &mut Documents, item: &Item| {
         let xot = documents.xot_mut();
         let attrs = xot.attributes(item.to_node()?);
         Ok(attrs.get(syspath).unwrap().clone())
      },
   )?;
   let query2 = DocumentsContext::queries().many(
      "//*[@SUBSYSTEM='hid']/*/*/self::device[@ID_INPUT='1' and matches(@key:SYSNAME, 'event\\d+')]",
      move |documents: &mut Documents, item: &Item| {
         let xot = documents.xot_mut();
         let attrs = xot.attributes(item.to_node()?);
         Ok(attrs.get(syspath).unwrap().clone())
      },
   )?;

   let (sender, mut receiver1) = tokio::sync::broadcast::channel(10);
   let mut receiver2 = sender.subscribe();
   let mut receiver3 = sender.subscribe();

   let local1 = LocalSet::new();
   local1.spawn_local(async move {
      loop {
         tree.ready().await?;
         sender.send(())?;
      }
      #[allow(unreachable_code)]
      Ok::<_, Box<dyn std::error::Error>>(())
   });

   let local2 = LocalSet::new();
   local2.spawn_local(async move {
      loop {
         receiver1.recv().await?;
         for item in query1.execute(&mut *handle1.get().await, *handle1.document_element())? {
            println!("match: {:?}", item);
         }
      }
      #[allow(unreachable_code)]
      Ok::<_, Box<dyn std::error::Error>>(())
   });

   local2.spawn_local(async move {
      loop {
         receiver2.recv().await?;
         for item in query2.execute(&mut *handle2.get().await, *handle2.document_element())? {
            println!("match: {:?}", item);
         }
      }
      #[allow(unreachable_code)]
      Ok::<_, Box<dyn std::error::Error>>(())
   });

   tokio::task::spawn(async move {
      loop {
         tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
         println!("fake multithreaded work 1");
      }
   });

   tokio::task::spawn(async move {
      loop {
         tokio::time::sleep(tokio::time::Duration::from_millis(1537)).await;
         println!("fake multithreaded work 2");
      }
   });

   tokio::task::spawn(async move {
      loop {
         receiver3.recv().await?;
         println!("fake multithreaded work 3");
      }
      #[allow(unreachable_code)]
      Ok::<_, tokio::sync::broadcast::error::RecvError>(())
   });

   select! {
      _ = local1 => {},
      _ = local2 => {},
   }

   println!("Something went wrong");

   Ok(())
}
