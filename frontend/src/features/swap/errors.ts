class InsufficientAccountBalanceError extends Error {
  requiredValue: bigint;

  constructor(symbol: string, requiredValue: bigint) {
    super(`Not enough ${symbol} to pay gas and fees`);
    this.requiredValue = requiredValue;
  }
}

export { InsufficientAccountBalanceError };
