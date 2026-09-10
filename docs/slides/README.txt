Place this bundle under:
  /Users/alpblkba/Documents/GitHub/laxity/docs/slides/

Expected repository paths used by the deck:
  ../diagrams/generated/architecture.svg
  ../diagrams/generated/runtime-sequence.svg
  ../diagrams/generated/experiment-flow.svg
  ../diagrams/generated/memory-contention.svg
  ../../assets/01-dashboard-wide.png
  ../../assets/04-dashboard-100x30.png

Render to PowerPoint with Marp CLI:
  cd /Users/alpblkba/Documents/GitHub/laxity
  npx @marp-team/marp-cli docs/slides/laxity_ces.md \
    --pptx \
    --html \
    --allow-local-files \
    -o laxity-ces.pptx

Render to PDF:
  npx @marp-team/marp-cli docs/slides/laxity_ces.md \
    --pdf \
    --html \
    --allow-local-files \
    -o laxity-ces.pdf

The CES/KIT artwork in ces-assets/ was extracted from the supplied CES PowerPoint template.
