const FORMATTER = new Intl.NumberFormat('en', {
  style: 'currency',
  currency: 'USD',
  notation: 'compact',
  maximumFractionDigits: 2,
});

function formatTvl(value: number) {
  return FORMATTER.format(value);
}

export { formatTvl };
