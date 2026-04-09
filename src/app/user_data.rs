use crate::lamp_dimmer::{DimmerChannelState, DimmerSettings, GammaCorrection, MAX_BRIGHTNESS};

use embassy_embedded_hal::adapter::BlockingAsync;
use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};
use esp_bootloader_esp_idf::partitions::{
    self, DataPartitionSubType, FlashRegion, PartitionTable, PartitionType,
};
use esp_storage::FlashStorage;
use static_cell::StaticCell;

type UserDataRegion = BlockingAsync<FlashRegion<'static, FlashStorage<'static>>>;
static FLASH_STORAGE: StaticCell<FlashStorage<'static>> = StaticCell::new();
static PARITION_TABLE_DATA: StaticCell<[u8; 0xC00]> = StaticCell::new();
static USER_DATA_REGION: StaticCell<UserDataRegion> = StaticCell::new();

static CURRENT_VERSION: u32 = 1; // Increment this if UserData structure changes in a non-compatible way
static USERDATA_MAGIC: u32 = 0x55_53_45_52; // "USER" in ASCII, used to verify valid data in flash

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserData {
    magic: u32,
    version: u32,
    pub dimmer_state: DimmerChannelState,
    pub dimmer_settings: DimmerSettings,
}

impl UserData {
    pub fn default() -> Self {
        Self {
            magic: USERDATA_MAGIC,
            version: CURRENT_VERSION,
            dimmer_state: DimmerChannelState {
                brightness: MAX_BRIGHTNESS,
                is_on: false,
            },
            dimmer_settings: DimmerSettings {
                perceived_zero_brightness: 0,
                perceived_full_brightness: MAX_BRIGHTNESS,
                gamma_correction: GammaCorrection::Linear,
            },
        }
    }
}

static USERDATA_OFFSET: u32 = 0;
pub struct UserDataStorage {
    _partition_table: PartitionTable<'static>,
    userdata_region: &'static mut UserDataRegion,
}

impl UserDataStorage {
    pub async fn read(&mut self) -> UserData {
        let mut buffer: [u8; size_of::<UserData>()] = [0u8; size_of::<UserData>()];
        let result = self
            .userdata_region
            .read(USERDATA_OFFSET, &mut buffer)
            .await;

        if result.is_err() {
            // Failed to read from flash, return default data
            return UserData::default();
        }

        let read_userdata: UserData = unsafe { core::mem::transmute(buffer) };
        if read_userdata.magic != USERDATA_MAGIC || read_userdata.version != CURRENT_VERSION {
            // Invalid data in flash, return default data
            return UserData::default();
        }

        read_userdata
    }

    pub async fn write(&mut self, data: &UserData) -> Result<(), ()> {
        let bytes: [u8; size_of::<UserData>()] = unsafe { core::mem::transmute(*data) };
        self.userdata_region
            .write(USERDATA_OFFSET, &bytes)
            .await
            .map_err(|_| ())?;
        Ok(())
    }
}

pub async fn initalize(flash: FlashStorage<'static>) -> UserDataStorage {
    let flash = FLASH_STORAGE.init(flash.multicore_auto_park());
    let data = PARITION_TABLE_DATA.init([0u8; 0xC00]); // Buffer for reading partition table

    let partion_table = partitions::read_partition_table(flash, data)
        .expect("Failed to read partition table from flash");

    // Find the NVS partition for storing user data
    let nvs_partition = partion_table
        .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
        .expect("Failed to find NVS partition")
        .expect("NVS partition not found in partition table");

    let userdata_region = nvs_partition.as_embedded_storage(flash);
    let userdata_async = USER_DATA_REGION.init(BlockingAsync::new(userdata_region));

    UserDataStorage {
        _partition_table: partion_table,
        userdata_region: userdata_async,
    }
}
