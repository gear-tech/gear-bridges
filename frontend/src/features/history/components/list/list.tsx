import { Button } from '@gear-js/vara-ui';
import { ReactNode } from 'react';

import styles from './list.module.scss';

type Props<T> = {
  items: T[] | undefined;
  hasMore: boolean;
  renderItem: (item: T) => ReactNode;
  fetchMore: () => void;

  skeleton: {
    rowsCount: number;
    isVisible: boolean;
    renderItem: () => ReactNode;
  };
};

function List<T>({ items, hasMore, renderItem, fetchMore, skeleton }: Props<T>) {
  // TODO: id as a key
  const renderItems = () => items?.map((item, index) => <li key={index}>{renderItem(item)}</li>);

  const renderSkeletonItems = () =>
    new Array(skeleton.rowsCount).fill(null).map((_, index) => <li key={index}>{skeleton.renderItem()}</li>);

  const isEmpty = !items?.length && !skeleton.isVisible;

  if (isEmpty)
    return (
      <div className={styles.notFound}>
        {/* TODO: add background placeholders */}
        <h3 className={styles.heading}>Oops, Nothing Found!</h3>
        <p className={styles.text}>It seems there are no such transactions on Vara Network Bridge.</p>
      </div>
    );

  return (
    <div>
      <ul className={styles.list}>
        {renderItems()}
        {skeleton.isVisible && renderSkeletonItems()}
      </ul>

      {hasMore && !skeleton.isVisible && (
        <Button text="Load More" size="small" color="grey" block onClick={fetchMore} className={styles.button} />
      )}
    </div>
  );
}

export { List };
