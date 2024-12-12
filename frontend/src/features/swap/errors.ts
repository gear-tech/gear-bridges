class InsufficientAccountBalanceError extends Error {
  balanceValue: bigint;
  requiredValue: bigint;

  constructor(symbol: string, balanceValue: bigint, requiredValue: bigint) {
    super(`Not enough ${symbol} to pay gas and fees`);
    this.balanceValue = balanceValue;
    this.requiredValue = requiredValue;
  }
}

export { InsufficientAccountBalanceError };
