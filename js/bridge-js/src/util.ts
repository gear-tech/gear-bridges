let _silent = true;

export const initLogger = (silent = true) => {
  _silent = silent;
};

export const logger = {
  info: (data: string) => {
    if (!_silent) {
      console.log(`bridge-js [info] ${data}`);
    }
  },
  error: (data: string) => {
    console.error(`bridge-js [error] ${data}`);
  },
};
