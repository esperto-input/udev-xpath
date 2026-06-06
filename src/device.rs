use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use tap::Pipe;
use udev::Device;

pub trait UdevDevice: Sized {
   fn syspath(&self) -> &str;

   fn parent(&self) -> Option<Self>;

   fn sysname(&self) -> &str;

   fn subsystem(&self) -> Option<&str>;

   fn driver(&self) -> Option<&str>;

   fn attributes(&self) -> impl Iterator<Item = (String, String)>;

   #[allow(unused)]
   fn properties(&self) -> impl Iterator<Item = (String, String)>;

   #[allow(unused)]
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
