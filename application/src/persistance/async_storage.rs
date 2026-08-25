
// Alloc
extern crate alloc;
use alloc::sync::Arc;

// Embassy and synchronization primitives for asynchronous operations
use embassy_futures::join::join;
use embassy_sync::{blocking_mutex::raw::{RawMutex}, rwlock::RwLock, watch::Receiver};
use embassy_time::{Duration, Instant};

// Sequential storage and cache management for asynchronous storage operations
use embedded_storage_async::nor_flash::{NorFlash};
use sequential_storage::{
    cache::{CacheImpl}, map::{MapStorage, PostcardValue},
};
use sequential_storage::map::Key;

/// Trait for types that can be stored in the asynchronous storage system.
/// Requires the PostcardValue marker trait for storage in the asynchronous storage system.
pub trait StorageData<'a, K : Key, const BUFF_SIZE: usize>: PostcardValue<'a> + Clone {
    /// The buffer size required for reading this type from sequential storage, aligned to 4 bytes.
    const BUFF_SIZE: usize;
    const KEY: K;
}

/// Manages app storage in the NVS partition of flash memory
pub struct AsyncStorage<M, K, F, C>
    where
        M: RawMutex,
        K: Key,
        F: NorFlash,
        C: CacheImpl<K>,
    {
    storage: Arc<RwLock<M, MapStorage<K, F, C>>>,
}

/// Represents errors that can occur during asynchronous storage operations.
#[derive(Debug, defmt::Format)]
pub enum AsyncStorageError<E> {
    StorageError(sequential_storage::Error<E>),
    StorageAlreadyInitialized,
}

impl<M, K, F, C> AsyncStorage<M, K, F, C>
    where
        M: RawMutex,
        K: Key,
        F: NorFlash,
        C: CacheImpl<K>,
{
    /// Creates a new instance of `AsyncStorage` with the provided `MapStorage`.
    pub fn new(storage : MapStorage<K, F, C>) -> Self {
        AsyncStorage {
            storage: Arc::new(RwLock::new(storage)),
        }
    }

    /// Reads a value from the storage.
    /// Returns `Ok(Some(value))` if the value exists, `Ok(None)` if it doesn't, and `Err` if an error occurs.
    /// The buffer provided must be at least `T::BUFF_SIZE` bytes long.
    /// Borrows the buffer for the duration of the lifetime 'a of the returned value.
    pub async fn read<'a, 'b : 'a, T, const BUFF_SIZE : usize>
        (&mut self, buffer : &'b mut [u8; BUFF_SIZE]) -> Result<Option<T>, AsyncStorageError<F::Error>> 
            where T: StorageData<'a, K, BUFF_SIZE> + 'a
    {
        let mut storage = self.storage.write().await;
        storage.fetch_item(buffer, &T::KEY).await.map_err(AsyncStorageError::StorageError)
    }

    /// Reads a value from the storage, returning the default value if the item doesn't exist.
    /// The buffer provided must be at least `T::BUFF_SIZE` bytes long.
    /// Borrows the buffer for the duration of the lifetime 'a of the returned value.
    pub async fn read_or_default<'a, T: StorageData<'a, K, BUFF_SIZE> + 'a, const BUFF_SIZE: usize>(&mut self, buffer : &'a mut [u8; BUFF_SIZE], default: T) -> Result<T, AsyncStorageError<F::Error>> {
        match self.read(buffer).await? {
            Some(value) => Ok(value),
            None => Ok(default),
        }
    }

    /// Writes a value to the storage, updating the underlying storage item.
    /// Doesn't take ownership of the buffer and allows it to be reused after the write operation.
    pub async fn write<'a, T: StorageData<'a, K, BUFF_SIZE> + 'a, const BUFF_SIZE: usize>(&mut self, buffer : &mut [u8; BUFF_SIZE], value: T) -> Result<(), AsyncStorageError<F::Error>> {
        let mut storage = self.storage.write().await;
        storage.store_item(buffer, &T::KEY, &value).await.map_err(AsyncStorageError::StorageError)
    }

    /// Requests an `ReactiveAsyncStorage` instance for the specified storage item type `T`.
    /// The `default` value is used if the item doesn't exist in the underlying storage.
    pub fn request<'a, T, const N: usize, const BUFF_SIZE: usize>(&self, receiver : Receiver<'a, M, T, N>) -> ReactiveAsyncStorage<'a, M, K, F, C, T, N, BUFF_SIZE>
        where
            T: StorageData<'a, K, BUFF_SIZE>, {
        ReactiveAsyncStorage::new(self.storage.clone(), receiver)
    }
}

/// Storage structure for managing access to a storage item of type T.
/// Do not set the `BUFF_SIZE` parameter manually;
/// it is automatically determined based on the size of the storage data type and the key.
pub struct ReactiveAsyncStorage<'a, M, K, F, C, T, const N: usize, const BUFFER_SIZE : usize>
    where
        F: NorFlash,
        K: Key,
        M: RawMutex,
        C: CacheImpl<K>,
        T: StorageData<'a, K, BUFFER_SIZE> {
    _marker: core::marker::PhantomData<&'a T>,
    receiver : Receiver<'a, M, T, N>,
    storage : Arc<RwLock<M, MapStorage<K, F, C>>>,
    last_write : embassy_time::Instant,
    write_buffer : [u8; BUFFER_SIZE],
}

impl<'a, M, K, F, C, T, const N: usize, const BUFFER_SIZE: usize> ReactiveAsyncStorage<'a, M, K, F, C, T, N, BUFFER_SIZE> 
    where 
        F: NorFlash,
        K: Key,
        M: RawMutex,
        C: CacheImpl<K>,
        T: StorageData<'a, K, BUFFER_SIZE> {

    pub(super) fn new(storage: Arc<RwLock<M, MapStorage<K, F, C>>>, receiver: Receiver<'a, M, T, N>) -> Self {
        ReactiveAsyncStorage {
            _marker: core::marker::PhantomData,
            receiver,
            storage,
            last_write: embassy_time::Instant::now(),
            write_buffer: [0u8; BUFFER_SIZE],
        }
    }

    // Starts the storage loop that listens for changes and persists them to the underlying storage.
    pub async fn storage_loop(mut self) -> Result<(), AsyncStorageError<F::Error>> {
        // Read the current value from storage or use the default if not present
        let mut receiver = self.receiver;

        // Start the storage loop to listen for changes and persist them
        let mut nvs_storage = self.storage.write().await;
        loop {
            // Time out to ensure writing every MAX_WAIT duration
            const WRITE_LIMIT: embassy_time::Duration = embassy_time::Duration::from_secs(5);

            let time_until_write = Instant::now()
                // Calculate duration since last write
                .checked_duration_since(self.last_write)
                // If the duration since last write is longer the write limit it will result in None
                .and_then(|duration| WRITE_LIMIT.checked_sub(duration))
                // If either subtraction results in None, it means an immediate write is needed.
                .unwrap_or(Duration::from_secs(0)); // Default to 0 for immediate write

            // Wait for a change and for the write timeout
            let (new_value, _) = join(receiver.changed(), embassy_time::Timer::after(time_until_write)).await;

            // Store the latest value to the underlying storage
            defmt::info!("Storing new value to async storage.");
            nvs_storage.store_item(&mut self.write_buffer, &T::KEY, &new_value).await
                .map_err(AsyncStorageError::StorageError)?;

            self.last_write = Instant::now();
        }
    }
}