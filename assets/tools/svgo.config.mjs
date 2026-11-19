// svgo settings for the lt artwork: the default preset, keeping the
// <title> and viewBox (the accessible name and scaling).
export default {
  multipass: true,
  plugins: [
    {
      name: "preset-default",
      params: { overrides: { removeTitle: false, removeViewBox: false } },
    },
  ],
};
