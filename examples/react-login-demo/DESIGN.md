---
name: Sakha AI Hub Design System
colors:
  background: "#0B0F1A"
  surface: "#131A2A"
  primary: "#6366F1"
  primary-hover: "#818CF8"
  accent: "#10B981"
  text-primary: "#E5E7EB"
  text-muted: "#94A3B8"
  border: "#1E293B"
typography:
  display:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 24px
    fontWeight: 700
  body:
    fontFamily: "Inter, system-ui, sans-serif"
    fontSize: 16px
    fontWeight: 400
spacing:
  xs: 4px
  sm: 8px
  md: 12px
  lg: 16px
  xl: 24px
  xxl: 32px
rounded:
  cards: 16px
  buttons: 10px
components:
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.cards}"
    border: "1px solid {colors.border}"
  button:
    backgroundColor: "{colors.primary}"
    textColor: "#FFFFFF"
    rounded: "{rounded.buttons}"
    padding: "{spacing.md} {spacing.xl}"
---

## Overview
Sakha AI Hub is a premium, modern dashboard designed for clarity and performance. The visual language uses deep dark tones to reduce eye strain, high-contrast typography for readability, and subtle gradients to convey a professional, tech-forward aesthetic.

## Colors
- **Background**: #0B0F1A (Deep charcoal base)
- **Surface**: #131A2A (Elevated cards)
- **Primary**: #6366F1 (Indigo brand color)
- **Accent**: #10B981 (Emerald for pricing/positive data)
- **Text**: #E5E7EB (Primary), #94A3B8 (Muted)

## Layout & Spacing
A consistent 8px-based grid system is used throughout. 
- Spacing: 4px, 8px, 12px, 16px, 24px, 32px.
- Components use `16px` for cards and `10px` for interactive elements to balance softness with purpose.

## Elevation & Depth
Depth is created through tonal contrast between the background and cards. Shadows are "double-layered" using subtle RGBA offsets to provide a soft, floating effect without feeling heavy.

## Component Inventory
- **StatCard**: High-level KPI visibility with animated icons and count-up values.
- **PriceBarChart**: Comparative horizontal visualization of provider input/output pricing.
- **DonutChart**: Circular distribution of pricing across the dataset.
- **ProviderCard**: Detailed service listing with mini-visualizations and interactive hover states.
- **SkeletonCard**: Shimmering placeholder state for loading feedback.
- **LoginCard**: Glassmorphic authentication portal with floating labels.

## Animation & Motion
- **Global Transitions**: 200ms ease-out for all hover and active states.
- **Aurora Background**: 30s-60s slow-moving radial gradients using `transform: translate()` for performance.
- **Entrance**: Staggered `fadeUp` animation (60ms delay increments).
- **Feedback**: 400ms shake for authentication errors; 800ms shimmer for skeletons.

## Chart Patterns
- **Grouped Bars**: Primary (input) and Accent (output) gradients on dark tracks.
- **Mini Price-Bars**: High-density horizontal bars within cards for at-a-glance comparison.
- **Donut Sweep**: SVG `stroke-dasharray` animation from 0 to target on mount.
