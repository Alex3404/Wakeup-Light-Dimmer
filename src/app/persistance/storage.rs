use embassy_embedded_hal::adapter::BlockingAsync;
use esp_bootloader_esp_idf::partitions::{self, DataPartitionSubType, PartitionType};

use esp_storage::{FlashStorage, FlashStorageError};
use sequential_storage::{
    cache::NoCache,
    map::{MapConfig, MapStorage, PostcardValue},
};

pub trait StorageData<'a>: PostcardValue<'a> + Default + Copy {
    const KEY: u8;
}

pub trait StorageRepositoryItem: PostcardValue<'static> + Default + Copy {}

pub trait StorageRepository<T> {}

/// Errors that can occur during user data storage operations
#[derive(Debug)]
pub enum StorageError {
    PartitionError(partitions::Error),
    StorageError(sequential_storage::Error<FlashStorageError>),
    PartitionNotFound,
    AlreadyInitialized,
}

/// Manages app storage in the NVS partition of flash memory
pub struct AppStorage {
    nvs_storage: MapStorage<u8, BlockingAsync<FlashStorage<'static>>, NoCache>,
}

impl AppStorage {
    pub fn new(flash: FlashStorage<'static>) -> Result<Self, StorageError> {
        // Buffer for reading partition table
        let mut partition_table_buffer = [0u8; 0xC00];

        let mut flash = flash.multicore_auto_park();
        let partition_table =
            partitions::read_partition_table(&mut flash, &mut partition_table_buffer)
                .map_err(StorageError::PartitionError)?;

        let partition_result = partition_table
            .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
            .map_err(StorageError::PartitionError)?;

        let nvs_partition = partition_result.ok_or(StorageError::PartitionNotFound)?;

        let nvs_range = nvs_partition.offset()..(nvs_partition.offset() + nvs_partition.len());
        let region = BlockingAsync::new(flash);
        let storage = MapStorage::new(region, MapConfig::new(nvs_range), NoCache::new());

        Ok(AppStorage {
            nvs_storage: storage,
        })
    }

    pub async fn read<'a, T: StorageData<'a>>(
        &mut self,
        buffer: &'a mut [u8],
    ) -> Result<Option<T>, StorageError> {
        self.nvs_storage
            .fetch_item::<T>(buffer, &T::KEY)
            .await
            .map_err(StorageError::StorageError)
    }

    pub async fn write<'a, T: StorageData<'a>>(
        &mut self,
        value: &T,
        buffer: &'a mut [u8],
    ) -> Result<(), StorageError> {
        self.nvs_storage
            .store_item(buffer, &T::KEY, value)
            .await
            .map_err(StorageError::StorageError)
    }
}
