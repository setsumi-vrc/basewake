//! Bluetooth LE access. `Radio` is the seam the controller talks to; `WinRadio` is the real
//! implementation on top of the Windows Runtime Bluetooth APIs.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::station::Mac;

/// A base station seen during a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub mac: Mac,
    pub name: String,
    pub rssi: i16,
}

/// An open connection to one base station's power characteristic.
pub trait Link: Send {
    fn read_power(&mut self) -> Result<u8>;
    fn write_power(&mut self, value: u8) -> Result<()>;
}

pub trait Radio: Send + Sync + 'static {
    fn connect(&self, mac: Mac) -> Result<Box<dyn Link>>;
    /// Listens for advertisements for `duration` and returns every Lighthouse V2 (`LHB-*`) seen.
    fn scan(&self, duration: Duration) -> Result<Vec<Discovered>>;
}

/// Name prefix that Lighthouse V2 base stations advertise.
pub const LHB_PREFIX: &str = "LHB-";

pub use win::WinRadio;

mod win {
    use super::*;
    use windows::Devices::Bluetooth::Advertisement::{
        BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEScanningMode,
    };
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristic, GattCommunicationStatus, GattDeviceService,
    };
    use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothLEDevice};
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::{DataReader, DataWriter};
    use windows::core::GUID;

    const PWR_SERVICE: GUID = GUID::from_u128(0x00001523_1212_efde_1523_785feabcd124);
    const PWR_CHARACTERISTIC: GUID = GUID::from_u128(0x00001525_1212_efde_1523_785feabcd124);

    /// Keeps a multithreaded COM apartment alive for the whole process, so WinRT calls work from
    /// any worker thread without each one initialising COM itself.
    fn ensure_mta() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            if let Err(e) = windows::Win32::System::Com::CoIncrementMTAUsage() {
                tracing::warn!("CoIncrementMTAUsage failed: {e}");
            }
        });
    }

    #[derive(Default)]
    pub struct WinRadio;

    impl WinRadio {
        pub fn new() -> Self {
            ensure_mta();
            WinRadio
        }
    }

    struct WinLink {
        device: BluetoothLEDevice,
        service: GattDeviceService,
        characteristic: GattCharacteristic,
    }

    impl Drop for WinLink {
        fn drop(&mut self) {
            // Closing releases the GATT session so Windows drops the BLE connection.
            let _ = self.service.Close();
            let _ = self.device.Close();
        }
    }

    fn check(status: GattCommunicationStatus, what: &str) -> Result<()> {
        if status == GattCommunicationStatus::Success {
            Ok(())
        } else if status == GattCommunicationStatus::Unreachable {
            bail!("{what}: device unreachable")
        } else if status == GattCommunicationStatus::AccessDenied {
            bail!("{what}: access denied")
        } else {
            bail!("{what}: GATT status {}", status.0)
        }
    }

    impl Link for WinLink {
        fn read_power(&mut self) -> Result<u8> {
            let r = self.characteristic.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?.join()?;
            check(r.Status()?, "read")?;
            let reader = DataReader::FromBuffer(&r.Value()?)?;
            if reader.UnconsumedBufferLength()? < 1 {
                bail!("read: empty value");
            }
            let mut b = [0u8; 1];
            reader.ReadBytes(&mut b)?;
            Ok(b[0])
        }

        fn write_power(&mut self, value: u8) -> Result<()> {
            let writer = DataWriter::new()?;
            writer.WriteBytes(&[value])?;
            let buf = writer.DetachBuffer()?;
            let r = self.characteristic.WriteValueWithResultAsync(&buf)?.join()?;
            check(r.Status()?, "write")
        }
    }

    impl Radio for WinRadio {
        fn connect(&self, mac: Mac) -> Result<Box<dyn Link>> {
            ensure_mta();
            let device = BluetoothLEDevice::FromBluetoothAddressAsync(mac.0)?
                .join()
                .map_err(|e| anyhow!("not found ({e})"))?;
            // Uncached forces Windows to actually connect instead of answering from its cache.
            let services =
                device.GetGattServicesForUuidWithCacheModeAsync(PWR_SERVICE, BluetoothCacheMode::Uncached)?.join()?;
            check(services.Status()?, "connect")?;
            let list = services.Services()?;
            if list.Size()? == 0 {
                bail!("power service not found — not a Lighthouse V2?");
            }
            let service = list.GetAt(0)?;
            let chars = service
                .GetCharacteristicsForUuidWithCacheModeAsync(PWR_CHARACTERISTIC, BluetoothCacheMode::Uncached)?
                .join()?;
            check(chars.Status()?, "characteristic")?;
            let clist = chars.Characteristics()?;
            if clist.Size()? == 0 {
                bail!("power characteristic not found");
            }
            let characteristic = clist.GetAt(0)?;
            Ok(Box::new(WinLink { device, service, characteristic }))
        }

        fn scan(&self, duration: Duration) -> Result<Vec<Discovered>> {
            ensure_mta();
            let found: Arc<Mutex<BTreeMap<u64, Discovered>>> = Arc::default();
            let watcher = BluetoothLEAdvertisementWatcher::new()?;
            // Active scanning also requests scan responses, which is where the name often lives.
            watcher.SetScanningMode(BluetoothLEScanningMode::Active)?;
            let sink = found.clone();
            watcher.Received(&TypedEventHandler::<
                BluetoothLEAdvertisementWatcher,
                BluetoothLEAdvertisementReceivedEventArgs,
            >::new(move |_, args| {
                let args = args.ok()?;
                let addr = args.BluetoothAddress()?;
                let name = args.Advertisement()?.LocalName()?.to_string();
                let rssi = args.RawSignalStrengthInDBm().unwrap_or(i16::MIN);
                let mut map = sink.lock().unwrap();
                match map.get_mut(&addr) {
                    Some(d) => {
                        if d.name.is_empty() && !name.is_empty() {
                            d.name = name;
                        }
                        d.rssi = d.rssi.max(rssi);
                    }
                    None => {
                        map.insert(addr, Discovered { mac: Mac(addr), name, rssi });
                    }
                }
                Ok(())
            }))?;
            watcher.Start()?;
            std::thread::sleep(duration);
            watcher.Stop()?;
            let map = found.lock().unwrap();
            Ok(map.values().filter(|d| d.name.starts_with(LHB_PREFIX)).cloned().collect())
        }
    }
}
