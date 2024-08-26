import { isUndefined } from '@/utils';
import React, { useEffect, useState } from 'react';

type FeeLoaderProps = {
  startTimestamp?: number;
  mortality?: number;
  size?: number;
  strokeWidth?: number;
  color?: string;
  onTimeEnd: () => void;
};

const PREFETCH_GAP = 1000;

const FeeLoader: React.FC<FeeLoaderProps> = ({
  startTimestamp,
  mortality,
  size = 24,
  strokeWidth = 4,
  color = '#0ed3a3',
  onTimeEnd,
}) => {
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const isStoped = !startTimestamp || !mortality;

  const [timeLeft, setTimeLeft] = useState<number>();

  useEffect(() => {
    if (isStoped) {
      return;
    }
    const endTimestamp = startTimestamp + mortality;

    if (endTimestamp - Date.now() > 0) {
      const timerId = setInterval(() => {
        const currentTimeLeft = endTimestamp - Date.now();
        setTimeLeft((prevTimeLeft) => (isUndefined(prevTimeLeft) ? mortality : currentTimeLeft));
        if (currentTimeLeft < PREFETCH_GAP) {
          clearInterval(timerId);
          setTimeLeft(undefined);
          onTimeEnd();
        }
      }, 1000);

      return () => clearInterval(timerId);
    }
  }, [isStoped, startTimestamp, mortality]);

  const progress = isStoped || !timeLeft ? undefined : -circumference * (1 - timeLeft / mortality);

  return (
    <svg width={size} height={size}>
      <circle stroke="#e6e6e6" fill="none" strokeWidth={strokeWidth} cx={size / 2} cy={size / 2} r={radius} />
      {progress && (
        <circle
          stroke={color}
          fill="none"
          strokeWidth={strokeWidth}
          cx={size / 2}
          cy={size / 2}
          r={radius}
          strokeDasharray={circumference}
          strokeDashoffset={progress}
          style={{
            transition: 'stroke-dashoffset 1s linear',
            transform: 'rotate(-90deg)',
            transformOrigin: '50% 50%',
          }}
        />
      )}
    </svg>
  );
};

export { FeeLoader };
