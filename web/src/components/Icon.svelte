<script lang="ts">
  // Every icon on the site goes through here, so they all share one size and stroke.
  import type { Component } from 'svelte'
  import type { LucideProps } from '@lucide/svelte'

  let {
    of: Glyph,
    label,
    spin = false,
  }: {
    of: Component<LucideProps>
    /** Accessible name; without it the icon is decorative. */
    label?: string
    spin?: boolean
  } = $props()
</script>

<Glyph
  size={16}
  strokeWidth={2}
  class={spin ? 'icon spin' : 'icon'}
  aria-hidden={label ? undefined : 'true'}
  aria-label={label}
  role={label ? 'img' : undefined}
/>

<style>
  :global(svg.icon) {
    flex: none;
    vertical-align: -0.15em;
  }
</style>
