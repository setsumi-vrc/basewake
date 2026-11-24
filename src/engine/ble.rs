//! Bluetooth LE access. `Radio` is the seam the controller talks to. On Windows the real one is
//! `WinRadio` (Windows Runtime APIs, connects straight by address); on Linux and macOS it is
//! `BtleRadio` (btleplug: BlueZ / CoreBluetooth). `BASEWAKE_BLE=btleplug` uses btleplug on
//! Windows too, to test that path.

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

/// The radio for this system.
pub fn default_radio() -> Arc<dyn Radio> {
    #[cfg(windows)]
    if std::env::var("BASEWAKE_BLE").as_deref() != Ok("btleplug") {
        return Arc::new(win::WinRadio::new());
    }
    Arc::new(btle::BtleRadio::new())
}

#[cfg(windows)]
pub use win::WinRadio;

#[cfg(windows)]
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

mod btle {
    use std::time::Instant;

    use btleplug::api::{Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType};
    use btleplug::platform::{Adapter, Manager, Peripheral};
    use tokio::runtime::Runtime;
    use uuid::Uuid;

    use super::*;

    const PWR_SERVICE: Uuid = Uuid::from_u128(0x00001523_1212_efde_1523_785feabcd124);
    const PWR_CHARACTERISTIC: Uuid = Uuid::from_u128(0x00001525_1212_efde_1523_785feabcd124);
    /// How long `connect` listens for a base station that the system has not seen yet.
    const FIND_TIMEOUT: Duration = Duration::from_secs(8);
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

    pub struct BtleRadio {
        rt: Arc<Runtime>,
        adapter: Mutex<Option<Adapter>>,
        /// Stations looking for themselves at once share one scan: it stops with the last one.
        scanners: Mutex<usize>,
    }

    impl BtleRadio {
        pub fn new() -> Self {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("bluetooth")
                .enable_all()
                .build()
                .expect("tokio runtime");
            BtleRadio { rt: Arc::new(rt), adapter: Mutex::new(None), scanners: Mutex::new(0) }
        }

        /// The first Bluetooth adapter, looked up once and kept.
        fn adapter(&self) -> Result<Adapter> {
            let mut slot = self.adapter.lock().unwrap();
            if let Some(a) = &*slot {
                return Ok(a.clone());
            }
            let a = self.rt.block_on(async {
                let manager = Manager::new().await?;
                manager.adapters().await?.into_iter().next().ok_or_else(|| anyhow!("no Bluetooth adapter"))
            })?;
            *slot = Some(a.clone());
            Ok(a)
        }

        async fn scan_start(&self, adapter: &Adapter) -> Result<()> {
            let first = {
                let mut n = self.scanners.lock().unwrap();
                *n += 1;
                *n == 1
            };
            if first {
                adapter.start_scan(ScanFilter::default()).await?;
            }
            Ok(())
        }

        async fn scan_stop(&self, adapter: &Adapter) {
            let last = {
                let mut n = self.scanners.lock().unwrap();
                *n = n.saturating_sub(1);
                *n == 0
            };
            if last {
                let _ = adapter.stop_scan().await;
            }
        }
    }

    /// The station id of a peripheral: its address, or on macOS (no addresses) its name.
    async fn id_of(p: &Peripheral) -> Option<(Mac, String)> {
        let props = p.properties().await.ok()??;
        let name = props.local_name.unwrap_or_default();
        let mac = if cfg!(target_os = "macos") { Mac::from_name(&name)? } else { Mac(address_u64(&props.address)) };
        Some((mac, name))
    }

    fn address_u64(a: &btleplug::api::BDAddr) -> u64 {
        a.into_inner().iter().fold(0u64, |v, b| (v << 8) | u64::from(*b))
    }

    async fn find(adapter: &Adapter, mac: Mac) -> Result<Option<Peripheral>> {
        for p in adapter.peripherals().await? {
            if id_of(&p).await.is_some_and(|(m, _)| m == mac) {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    struct BtleLink {
        rt: Arc<Runtime>,
        peripheral: Peripheral,
        characteristic: Characteristic,
    }

    impl Drop for BtleLink {
        fn drop(&mut self) {
            let _ = self.rt.block_on(self.peripheral.disconnect());
        }
    }

    impl Link for BtleLink {
        fn read_power(&mut self) -> Result<u8> {
            let v = self.rt.block_on(self.peripheral.read(&self.characteristic)).map_err(|e| anyhow!("read: {e}"))?;
            v.first().copied().ok_or_else(|| anyhow!("read: empty value"))
        }

        fn write_power(&mut self, value: u8) -> Result<()> {
            self.rt
                .block_on(self.peripheral.write(&self.characteristic, &[value], WriteType::WithResponse))
                .map_err(|e| anyhow!("write: {e}"))
        }
    }

    impl Radio for BtleRadio {
        fn connect(&self, mac: Mac) -> Result<Box<dyn Link>> {
            let adapter = self.adapter()?;
            let rt = self.rt.clone();
            let (peripheral, characteristic) = self.rt.block_on(async {
                // btleplug only connects to peripherals the system has seen: listen until it shows up.
                let mut p = find(&adapter, mac).await?;
                if p.is_none() {
                    self.scan_start(&adapter).await?;
                    let start = Instant::now();
                    while p.is_none() && start.elapsed() < FIND_TIMEOUT {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        p = find(&adapter, mac).await.unwrap_or(None);
                    }
                    self.scan_stop(&adapter).await;
                }
                let p = p.ok_or_else(|| anyhow!("not found (no advertisement seen)"))?;
                if !p.is_connected().await.unwrap_or(false) {
                    tokio::time::timeout(CONNECT_TIMEOUT, p.connect())
                        .await
                        .map_err(|_| anyhow!("connect timed out: device unreachable"))??;
                }
                p.discover_services().await?;
                let c = p
                    .characteristics()
                    .into_iter()
                    .find(|c| c.uuid == PWR_CHARACTERISTIC && c.service_uuid == PWR_SERVICE)
                    .ok_or_else(|| anyhow!("power service not found — not a Lighthouse V2?"))?;
                anyhow::Ok((p, c))
            })?;
            Ok(Box::new(BtleLink { rt, peripheral, characteristic }))
        }

        fn scan(&self, duration: Duration) -> Result<Vec<Discovered>> {
            let adapter = self.adapter()?;
            self.rt.block_on(async {
                self.scan_start(&adapter).await?;
                tokio::time::sleep(duration).await;
                self.scan_stop(&adapter).await;
                let mut found: BTreeMap<Mac, Discovered> = BTreeMap::new();
                for p in adapter.peripherals().await? {
                    let Some((mac, name)) = id_of(&p).await else { continue };
                    if !name.starts_with(LHB_PREFIX) {
                        continue;
                    }
                    let rssi = p.properties().await.ok().flatten().and_then(|x| x.rssi).unwrap_or(i16::MIN);
                    found.insert(mac, Discovered { mac, name, rssi });
                }
                Ok(found.into_values().collect())
            })
        }
    }
}
