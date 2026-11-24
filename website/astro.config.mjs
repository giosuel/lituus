// Lituus website (Astro + Starlight). See README.md in this directory.
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { remarkBaseLinks } from './plugins/remark-base-links.mjs';
import { BASE, SITE } from './site.config.mjs';

export default defineConfig({
  // Set in site.config.mjs (the only place that knows the base path).
  site: SITE,
  base: BASE,
  markdown: {
    remarkPlugins: [remarkBaseLinks(BASE)],
  },
  integrations: [
    starlight({
      title: 'Lituus',
      description:
        'The settings app for Hyprland and other desktops without one: network, Bluetooth, displays, sound, input, wallpaper, apps and more, written in Rust.',
      logo: { src: './src/assets/mark.svg' },
      favicon: '/favicon.svg',
      customCss: ['./src/styles/custom.css'],
      components: {
        ThemeProvider: './src/components/ThemeProvider.astro',
        ThemeSelect: './src/components/ThemeSelect.astro',
      },
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/giosuel/lituus' }],
      editLink: {
        baseUrl: 'https://github.com/giosuel/lituus/edit/main/website/',
      },
      lastUpdated: false,
      sidebar: [
        { label: 'Installing', link: '/install/' },
        {
          label: 'Guide',
          // In the app's order: setting up, then the pages as its sidebar lists them.
          items: [
            'guide/hyprland',
            'guide/window',
            'guide/network',
            'guide/bluetooth',
            'guide/displays',
            'guide/sound',
            'guide/input',
            'guide/wallpaper',
            'guide/apps',
            'guide/default-apps',
            'guide/system',
            'guide/settings',
          ],
        },
        { label: 'Development', link: '/development/' },
      ],
    }),
  ],
});
