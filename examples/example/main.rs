use std::fs;
use udev_xpath::AsyncTree;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
   // let mut tree: UdevTree<Device> = UdevTree::new_raw();
   // let mut connected_devices = Enumerator::new()?;
   // let query = tree
   //    .compile_query("//*[@SUBSYSTEM='hid']/*/*/self::device[@ID_INPUT='1' and matches(@key:SYSNAME, 'event\\d+')]")?;
   // connected_devices.match_subsystem("input")?;
   //
   // for dev in connected_devices.scan_devices()? {
   //    println!("Got new device!");
   //    tree.attach(dev.clone());
   // }
   // for dev in connected_devices.scan_devices()? {
   //    tree.attach(dev.clone());
   //    println!("double device!");
   // }
   // println!("finished!");
   // for item in tree.match_query(&query)? {
   //    println!("match: {:?}", item);
   // }

   let mut tree = AsyncTree::new().await?;
   while let Ok(()) = tree.ready().await {
      fs::write("output.xml", tree.to_string().await?)?
   }
   Ok(())

   // select! {
   //    e = async {
   //       loop {
   //          tokio::time::sleep(std::time::Duration::from_secs(5)).await;
   //          if let Err(e) = tree.read().await.serialize().and_then(|s| Ok(fs::write("output.xml",s)?)){
   //             break e;
   //          }
   //       }
   //    } => { Err(e.into()) },
   //    _ = handle => { Ok(()) }
   // }
}


/*pub fn compile_boolean_query(&self, query_str: &str) -> XeeResult<OneQuery<bool, impl Convert<bool> + use<D>>> {
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
}*/