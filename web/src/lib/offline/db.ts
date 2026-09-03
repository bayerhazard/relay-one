const DB_NAME = "relay-offline";
const DB_VERSION = 1;

let dbPromise: Promise<IDBDatabase> | null = null;

function openDB(): Promise<IDBDatabase> {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains("bodies")) {
        const store = db.createObjectStore("bodies", { keyPath: "key" });
        store.createIndex("account", "accountId");
        store.createIndex("accessedAt", "accessedAt");
      }
      if (!db.objectStoreNames.contains("outbox")) {
        db.createObjectStore("outbox", { keyPath: "id", autoIncrement: true });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
  return dbPromise;
}

function tx<T>(storeName: string, mode: IDBTransactionMode, fn: (store: IDBObjectStore) => IDBRequest<T> | void): Promise<T> {
  return openDB().then(
    (db) =>
      new Promise<T>((resolve, reject) => {
        const t = db.transaction(storeName, mode);
        const store = t.objectStore(storeName);
        let result: T;
        const req = fn(store);
        if (req) {
          req.onsuccess = () => {
            result = req.result;
          };
        }
        t.oncomplete = () => resolve(result);
        t.onerror = () => reject(t.error);
        t.onabort = () => reject(t.error);
      })
  );
}

export function idbGet<T>(store: string, key: IDBValidKey): Promise<T | undefined> {
  return tx<T | undefined>(store, "readonly", (s) => s.get(key) as IDBRequest<T | undefined>);
}

export function idbPut(store: string, value: unknown): Promise<void> {
  return tx<void>(store, "readwrite", (s) => {
    s.put(value as any);
  });
}

export function idbDelete(store: string, key: IDBValidKey): Promise<void> {
  return tx<void>(store, "readwrite", (s) => {
    s.delete(key);
  });
}

export function idbGetAll<T>(store: string): Promise<T[]> {
  return tx<T[]>(store, "readonly", (s) => s.getAll() as IDBRequest<T[]>);
}

export function idbCount(store: string): Promise<number> {
  return tx<number>(store, "readonly", (s) => s.count() as IDBRequest<number>);
}

export function idbClear(store: string): Promise<void> {
  return tx<void>(store, "readwrite", (s) => {
    s.clear();
  });
}

export function idbGetAllByIndex<T>(store: string, index: string, value: IDBValidKey): Promise<T[]> {
  return tx<T[]>(store, "readonly", (s) => s.index(index).getAll(value) as IDBRequest<T[]>);
}
