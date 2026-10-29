import type { GraphComparison } from './graphComparison';

export interface SavedDataset { id: string; name: string; origin: 'generated' | 'imported' | 'recorded'; capture: GraphComparison }

async function database() {
  return new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('microblossom-graph-workbench', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('datasets', { keyPath: 'id' });
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export async function savedDatasets(): Promise<SavedDataset[]> {
  const db = await database();
  try { return await new Promise((resolve, reject) => {
    const request = db.transaction('datasets').objectStore('datasets').getAll();
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  }); } finally { db.close(); }
}

export async function saveDataset(dataset: SavedDataset) {
  const db = await database();
  try { await new Promise<void>((resolve, reject) => {
    const transaction = db.transaction('datasets', 'readwrite');
    transaction.objectStore('datasets').put(dataset);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error);
    transaction.onabort = () => reject(transaction.error);
  }); } finally { db.close(); }
}
