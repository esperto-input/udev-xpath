use std::fs;
use udev_xpath::{DocumentsContext, UdevTree};
use xee_xpath::{Documents, Item, Query};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
   let mut tree = UdevTree::builder().subsystems(["input", "hid"]).build().await?;

   let handle = tree.get_documents_context();
   let syspath = handle.name_ids().syspath;
   let query = DocumentsContext::queries().many(
      "//*[@SUBSYSTEM='hid']/*/*/self::device[@ID_INPUT='1' and matches(@key:SYSNAME, 'event\\d+')]",
      move |documents: &mut Documents, item: &Item| {
         let xot = documents.xot_mut();
         let attrs = xot.attributes(item.to_node()?);
         Ok(attrs.get(syspath).unwrap().clone())
      },
   )?;

   println!("Created tree!");
   for item in query.execute(&mut *handle.get().await, *handle.document_element())? {
      println!("match: {:?}", item);
   }
   println!("Waiting for devices...");

   while let Ok(()) = tree.ready().await {
      println!("Tree changed!");
      fs::write("output.xml", handle.to_string().await?)?;
      for item in query.execute(&mut *handle.get().await, *handle.document_element())? {
         println!("match: {:?}", item);
      }
      println!();
   }

   println!("Something went wrong in libudev");

   Ok(())
}
