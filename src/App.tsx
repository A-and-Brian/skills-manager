import { RouterProvider } from "@tanstack/react-router";
import { Toaster } from "sonner";
import { ThemeProvider, useThemeContext } from "./context/ThemeContext";
import { router } from "./router";

function ThemedToaster() {
  const { resolvedTheme } = useThemeContext();
  return (
    <Toaster
      theme={resolvedTheme}
      position="bottom-right"
      toastOptions={{
        style: {
          background: "var(--color-surface)",
          border: "1px solid var(--color-border)",
          color: "var(--color-text-primary)",
        },
      }}
    />
  );
}

function App() {
  return (
    <ThemeProvider>
      <RouterProvider router={router} />
      <ThemedToaster />
    </ThemeProvider>
  );
}

export default App;
