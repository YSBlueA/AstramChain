# Astram Blockchain Tip Recovery Tool
# This script helps recover the chain tip by scanning the database

Write-Host "🔍 Astram Chain Tip Recovery Tool" -ForegroundColor Cyan
Write-Host ""

$dbPath = "blockchain_data"

if (-not (Test-Path $dbPath)) {
    Write-Host "❌ Database not found at: $dbPath" -ForegroundColor Red
    exit 1
}

Write-Host "📊 Database found. Manual recovery options:" -ForegroundColor Yellow
Write-Host ""
Write-Host "Option 1: Let the node rebuild the tip automatically"
Write-Host "  - Add a tip recovery function to scan all blocks"
Write-Host "  - Find the block with highest valid work"
Write-Host ""
Write-Host "Option 2: Manual tip reset to specific block"
Write-Host "  - Requires knowing the correct tip hash"
Write-Host ""
Write-Host "Would you like to:" -ForegroundColor Cyan
Write-Host "1. Add automatic tip recovery to the code"
Write-Host "2. Check current tip status"
Write-Host ""
