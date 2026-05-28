#![feature(iter_collect_into)]

use crate::udev_tree::UdevTree;
use std::fs;
use udev::{Device, Enumerator};

pub mod udev_tree;

fn main() -> Result<(), Box<dyn std::error::Error>> {
   let mut tree: UdevTree<Device> = UdevTree::new();
   let mut connected_devices = Enumerator::new()?;
   let query = tree
      .compile_query("//*[@SUBSYSTEM='hid']/*/*/self::device[@ID_INPUT='1' and matches(@key:SYSNAME, 'event\\d+')]")?;
   connected_devices.match_subsystem("input")?;
   for dev in connected_devices.scan_devices()? {
      println!("Got new device!");
      tree.attach(dev.clone());
   }
   for dev in connected_devices.scan_devices()? {
      tree.attach(dev.clone());
      println!("double device!");
   }
   println!("finished!");
   for item in tree.match_query(&query)? {
      println!("match: {:?}", item);
   }
   fs::write("output.xml", tree.serialize()?)?;
   Ok(())
}

#[cfg(test)]
mod tests {
   use super::*;
   use std::collections::HashMap;
   use udev_tree::{DummyDevice, UdevTree};

   #[test]
   fn test_udev_tree_basic() {
      let mut tree = UdevTree::new();

      let mut root_attrs = HashMap::new();
      root_attrs.insert("SUBSYSTEM".to_string(), "pci".to_string());

      let root_device = DummyDevice {
         syspath: "/sys/devices/pci0000:00".to_string(),
         sysname: "pci0000:00".to_string(),
         driver: None,
         subsystem: None,
         properties: root_attrs,
         attributes: Default::default(),
         parent: None,
      };

      let mut child_attrs = HashMap::new();
      child_attrs.insert("SUBSYSTEM".to_string(), "usb".to_string());
      child_attrs.insert("ID_VENDOR".to_string(), "1234".to_string());

      let child_device = DummyDevice {
         syspath: "/sys/devices/pci0000:00/usb1".to_string(),
         sysname: "usb1".to_string(),
         driver: None,
         subsystem: None,
         properties: child_attrs,
         attributes: Default::default(),
         parent: Some(Box::new(root_device.clone())),
      };

      tree.attach(child_device.clone());

      // Let's test a query.
      // We query if the child node satisfies `self::device[@SUBSYSTEM="usb"]`
      let result = tree
         .compile_and_test_boolean_query(&child_device.syspath, "boolean(self::device[@SUBSYSTEM='usb'])")
         .unwrap();
      assert!(result, "Device should match its own subsystem attribute");

      // We query if the child node has an ancestor with SUBSYSTEM pci
      let result2 = tree
         .compile_and_test_boolean_query(&child_device.syspath, "boolean(ancestor::device[@SUBSYSTEM='pci'])")
         .unwrap();
      assert!(result2, "Device should have a PCI ancestor");

      // Negative test
      let result3 = tree
         .compile_and_test_boolean_query(&child_device.syspath, "boolean(self::device[@SUBSYSTEM='pci'])")
         .unwrap();
      assert!(!result3, "Device itself is not PCI");

      // Detach
      tree.detach(&child_device.syspath);
      let result4 = tree.compile_and_test_boolean_query(&child_device.syspath, "boolean(self::device)");
      assert!(result4.unwrap_or(false) == false, "Device should be detached");
   }

   #[test]
   fn test_udev_tree_basic2() {
      let mut tree = UdevTree::new();

      let mut root_attrs = HashMap::new();
      root_attrs.insert("SUBSYSTEM".to_string(), "pci".to_string());

      let root_device = DummyDevice {
         syspath: "/sys/devices/pci0000:00".to_string(),
         sysname: "pci0000:00".to_string(),
         driver: None,
         subsystem: None,
         properties: root_attrs,
         attributes: Default::default(),
         parent: None,
      };

      let mut child_attrs = HashMap::new();
      child_attrs.insert("SUBSYSTEM".to_string(), "usb".to_string());
      child_attrs.insert("ID_VENDOR".to_string(), "1234".to_string());

      let child_device = DummyDevice {
         syspath: "/sys/devices/pci0000:00/usb1".to_string(),
         sysname: "usb1".to_string(),
         driver: None,
         subsystem: None,
         properties: child_attrs,
         attributes: Default::default(),
         parent: Some(Box::new(root_device.clone())),
      };

      tree.attach(child_device.clone());

      // Let's test a query.
      // We query if the child node satisfies `self::device[@SUBSYSTEM="usb"]`
      let result = tree
         .compile_and_test_boolean_query(&child_device.syspath, "boolean(self::device[@SUBSYSTEM='usb'])")
         .unwrap();
      assert!(result, "Device should match its own subsystem attribute");

      // We query if the child node has an ancestor with SUBSYSTEM pci
      let result2 = tree
         .compile_and_test_boolean_query(&child_device.syspath, "boolean(ancestor::device[@SUBSYSTEM='pci'])")
         .unwrap();
      assert!(result2, "Device should have a PCI ancestor");

      // Negative test
      let result3 = tree
         .compile_and_test_boolean_query(&child_device.syspath, "boolean(self::device[@SUBSYSTEM='pci'])")
         .unwrap();
      assert!(!result3, "Device itself is not PCI");

      // Detach
      tree.detach(&child_device.syspath);
      let result4 = tree.compile_and_test_boolean_query(&child_device.syspath, "boolean(self::device)");
      assert!(result4.unwrap_or(false) == false, "Device should be detached");
   }

   /// Helper to create a Device with given syspath, name, attributes, and optional parent.
   fn make_device(syspath: &str, name: &str, attrs: Vec<(&str, &str)>, parent: Option<DummyDevice>) -> DummyDevice {
      let mut attributes = HashMap::new();
      for (k, v) in attrs {
         attributes.insert(k.to_string(), v.to_string());
      }
      DummyDevice {
         syspath: syspath.to_string(),
         sysname: name.to_string(),
         driver: None,
         subsystem: None,
         properties: attributes,
         attributes: Default::default(),
         parent: parent.map(Box::new),
      }
   }

   /// Test that attaching a deep device auto-populates all missing ancestors top-down.
   #[test]
   fn test_ancestry_auto_population() {
      let mut tree = UdevTree::new();

      let pci = make_device(
         "/sys/devices/pci0000:00",
         "pci0000:00",
         vec![("SUBSYSTEM", "pci")],
         None,
      );
      let usb_bus = make_device(
         "/sys/devices/pci0000:00/0000:00:14.0",
         "0000:00:14.0",
         vec![("SUBSYSTEM", "pci"), ("DRIVER", "xhci_hcd")],
         Some(pci.clone()),
      );
      let usb_root = make_device(
         "/sys/devices/pci0000:00/0000:00:14.0/usb1",
         "usb1",
         vec![("SUBSYSTEM", "usb"), ("DEVTYPE", "usb_device")],
         Some(usb_bus.clone()),
      );
      let leaf = make_device(
         "/sys/devices/pci0000:00/0000:00:14.0/usb1/1-1",
         "1-1",
         vec![("SUBSYSTEM", "usb"), ("ID_VENDOR", "045e")],
         Some(usb_root.clone()),
      );

      // Attach only the leaf — all ancestors should be created automatically.
      tree.attach(leaf.clone());

      // The leaf should be queryable.
      assert!(
         tree
            .compile_and_test_boolean_query(&leaf.syspath, "boolean(self::device[@ID_VENDOR='045e'])")
            .unwrap()
      );

      // All intermediate ancestors should exist and be queryable.
      assert!(
         tree
            .compile_and_test_boolean_query(&usb_root.syspath, "boolean(self::device[@DEVTYPE='usb_device'])")
            .unwrap()
      );
      assert!(
         tree
            .compile_and_test_boolean_query(&usb_bus.syspath, "boolean(self::device[@DRIVER='xhci_hcd'])")
            .unwrap()
      );
      assert!(
         tree
            .compile_and_test_boolean_query(&pci.syspath, "boolean(self::device[@SUBSYSTEM='pci'])")
            .unwrap()
      );

      // Ancestor axis queries from the leaf should work.
      assert!(
         tree
            .compile_and_test_boolean_query(&leaf.syspath, "boolean(ancestor::device[@DRIVER='xhci_hcd'])")
            .unwrap()
      );
      assert!(
         tree
            .compile_and_test_boolean_query(&leaf.syspath, "boolean(ancestor-or-self::device[@SUBSYSTEM='usb'])")
            .unwrap()
      );
   }

   /// Test that re-attaching an existing device updates its attributes without duplication.
   #[test]
   fn test_reattach_updates_attributes() {
      let mut tree = UdevTree::new();

      let dev = make_device(
         "/sys/devices/virtual/net/eth0",
         "eth0",
         vec![("SUBSYSTEM", "net"), ("INTERFACE", "eth0")],
         None,
      );
      tree.attach(dev);

      // Re-attach with updated attributes.
      let dev_updated = make_device(
         "/sys/devices/virtual/net/eth0",
         "eth0",
         vec![("SUBSYSTEM", "net"), ("INTERFACE", "eth0"), ("OPERSTATE", "up")],
         None,
      );
      tree.attach(dev_updated);

      assert!(
         tree
            .compile_and_test_boolean_query(
               "/sys/devices/virtual/net/eth0",
               "boolean(self::device[@OPERSTATE='up'])"
            )
            .unwrap()
      );
      // Original attribute should still be present.
      assert!(
         tree
            .compile_and_test_boolean_query(
               "/sys/devices/virtual/net/eth0",
               "boolean(self::device[@INTERFACE='eth0'])"
            )
            .unwrap()
      );
   }

   /// Test detaching a mid-tree node removes it and its descendants from queries.
   #[test]
   fn test_detach_mid_tree() {
      let mut tree = UdevTree::new();

      let root = make_device(
         "/sys/devices/platform",
         "platform",
         vec![("SUBSYSTEM", "platform")],
         None,
      );
      let mid = make_device(
         "/sys/devices/platform/i2c-0",
         "i2c-0",
         vec![("SUBSYSTEM", "i2c")],
         Some(root.clone()),
      );
      let leaf = make_device(
         "/sys/devices/platform/i2c-0/0-0050",
         "0-0050",
         vec![("SUBSYSTEM", "i2c"), ("DRIVER", "at24")],
         Some(mid.clone()),
      );

      tree.attach(leaf.clone());

      // Detach the middle node — should remove mid and its child (leaf) from the XML tree.
      tree.detach(&mid.syspath);

      // Mid node should no longer be queryable.
      assert!(
         !tree
            .compile_and_test_boolean_query(&mid.syspath, "boolean(self::device)")
            .unwrap_or(false)
      );

      // Root should still exist.
      assert!(
         tree
            .compile_and_test_boolean_query(&root.syspath, "boolean(self::device[@SUBSYSTEM='platform'])")
            .unwrap()
      );
   }

   /// Test attaching into a partially pre-existing tree (ancestor already present).
   #[test]
   fn test_attach_with_existing_ancestor() {
      let mut tree = UdevTree::new();

      let root = make_device(
         "/sys/devices/pci0000:00",
         "pci0000:00",
         vec![("SUBSYSTEM", "pci")],
         None,
      );
      tree.attach(root.clone());

      // Attach a grandchild whose parent (mid) doesn't exist yet, but grandparent (root) does.
      let mid = make_device(
         "/sys/devices/pci0000:00/0000:00:1f.0",
         "0000:00:1f.0",
         vec![("SUBSYSTEM", "pci"), ("CLASS", "bridge")],
         Some(root.clone()),
      );
      let leaf = make_device(
         "/sys/devices/pci0000:00/0000:00:1f.0/snd0",
         "snd0",
         vec![("SUBSYSTEM", "sound")],
         Some(mid.clone()),
      );

      tree.attach(leaf.clone());

      // Mid should have been auto-created.
      assert!(
         tree
            .compile_and_test_boolean_query(&mid.syspath, "boolean(self::device[@CLASS='bridge'])")
            .unwrap()
      );
      // Leaf should be a descendant of root.
      assert!(
         tree
            .compile_and_test_boolean_query(&leaf.syspath, "boolean(ancestor::device[@SUBSYSTEM='pci'])")
            .unwrap()
      );
   }
}
