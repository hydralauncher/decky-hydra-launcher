import deckyPlugin from "@decky/rollup";
import scss from "rollup-plugin-scss";
import sass from "sass";

export default deckyPlugin({
  // Add your extra Rollup options here
  plugins: [
    scss({
      output: false,
      sourceMap: false,
      include: ["src/styles/**/*.scss", "src/styles/**/*.sass"],
      watch: "src/styles",
      sass: sass,
    }),
  ],
});
