use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_sync::blocking_mutex::raw::RawMutex;
use esp_storage::{FlashStorage, FlashStorageError};
use esp_bootloader_esp_idf::partitions::{self, PartitionType, DataPartitionSubType};
use sequential_storage::map::{Key, MapConfig, MapStorage};
use sequential_storage::cache::{Cache, Uncached};
use super::AsyncStorage;

/// Errors that can occur during user data storage operations
#[derive(Debug, defmt::Format)]
pub enum StorageError {
    PartitionError(partitions::Error),
    StorageError(sequential_storage::Error<FlashStorageError>),
    PartitionNotFound,
    AlreadyInitialized,
}

type EspDriver<'a, K, M> = AsyncStorage<M, K, BlockingAsync<FlashStorage<'a>>, Cache<Uncached, Uncached, Uncached, K>>;

pub fn async_nvs_storage<'a, K : Key, M : RawMutex>(mut flash: FlashStorage<'a>) -> Result<EspDriver<'a, K, M>, StorageError> {
    
    // Buffer for reading partition table
    let mut partition_table_buffer = [0u8; 0xC00];

    let partition_table =
        partitions::read_partition_table(&mut flash, &mut partition_table_buffer)
            .map_err(StorageError::PartitionError)?;

    let partition_result = partition_table
        .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
        .map_err(StorageError::PartitionError)?;

    let nvs_partition = partition_result.ok_or(StorageError::PartitionNotFound)?;

    let nvs_end = nvs_partition.offset().checked_add(nvs_partition.len()).ok_or(StorageError::PartitionNotFound)?;
    let nvs_range = nvs_partition.offset()..nvs_end;
    let region = BlockingAsync::new(flash); 

    let storage = MapStorage::new(region, MapConfig::new(nvs_range), Cache::new_uncached());

    Ok(AsyncStorage::new(storage))
}