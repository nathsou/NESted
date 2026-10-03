export default {
  base: './',
  build: { target: 'es2022', outDir: 'dist' },
  worker: { format: 'es' },
  server: { port: 5173, strictPort: true },
};
