export const mapValues = <K, V>(map: Map<K, V>): Array<V> => Array.from(map.values());

export const mapKeys = <K, V>(map: Map<K, V>): Array<K> => Array.from(map.keys());

export const setValues = <V>(set: Set<V>): Array<V> => Array.from(set.values());
