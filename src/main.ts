import { createApp } from "vue";
import App from "./App.vue";
import { i18n, applyDirection } from "./i18n";
import "./styles.css";

applyDirection(i18n.global.locale.value);
createApp(App).use(i18n).mount("#app");
