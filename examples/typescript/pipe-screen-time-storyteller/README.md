# Screen Time Storyteller

## Overview

Screen Time Storyteller is an AI-powered pipe for Screenpipe that generates engaging, personalized narratives based on your daily screen activities. It uses data from your device usage to create a fun, slightly sassy diary entry, complete with insights and advice for better digital habits.

## Features

- 📊 Analyzes your screen time data collected by Screenpipe
- 🤖 Uses AI to generate personalized narratives
- 😎 Provides a fun, sassy perspective on your digital habits
- 💡 Offers insights and advice for better screen time management
- 📝 Creates daily summaries as GitHub gists
- 🔄 Supports multiple AI providers (Claude, OpenAI, Ollama)

## Prerequisites

- A working Screenpipe installation
- API keys for your chosen AI provider (Claude, OpenAI, or Ollama)
- GitHub Personal Access Token (for creating gists)

## Setup

1. Ensure you have Screenpipe installed and running.

2. Copy the `screen-time-storyteller` folder into your Screenpipe pipes directory:
   ```
   cp -r screen-time-storyteller /path/to/your/screenpipe/pipes/
   ```

3. Edit the `pipe.json` file in the `screen-time-storyteller` folder with your API keys, GitHub token, and preferences:
   ```json
   {
     "name": "Screen Time Storyteller",
     "version": "1.0.0",
     "description": "Generates a narrative summary of your day based on your screen activities",
     "author": "Your Name",
     "fields": [
       {
         "name": "aiProvider",
         "value": "your_chosen_provider"
       },
       {
         "name": "models",
         "value": {
           "claude": "claude-3-5-sonnet-20240620",
           "openai": "gpt-4-turbo-preview",
           "ollama": "llama3.1"
         }
       },
       {
         "name": "apiKeys",
         "value": {
           "claude": "your_claude_api_key",
           "openai": "your_openai_api_key"
         }
       },
       {
         "name": "pageSize",
         "value": 1000
       },
       {
         "name": "contentType",
         "value": "ocr"
       },
       {
         "name": "github",
         "value": {
           "personalAccessToken": "your_github_personal_access_token"
         }
       }
     ]
   }
   ```

## Usage

The Screen Time Storyteller pipe will run automatically as part of your Screenpipe workflow. It generates a narrative summary every hour based on your screen activities from the past 24 hours.

You can find the generated summaries in two places:

1. A JSON file in the `.screenpipe/pipes/screen-time-storyteller/` directory, named with the current date (e.g., `2024-10-06-narrative-summary.json`).
2. A GitHub gist, which you can access through your GitHub account.

## Configuration

You can adjust the pipe's behavior by modifying the `pipe.json` file:

- `aiProvider`: Choose between "claude", "openai", or "ollama"
- `models`: Specify the model to use for each provider
- `apiKeys`: Add your API keys for Claude and OpenAI
- `pageSize`: Set the number of screen time entries to analyze
- `contentType`: Choose between "ocr", "audio", or "all"
- `github`: Add your GitHub Personal Access Token

## Troubleshooting

If you encounter any issues:

1. Check Screenpipe's logs for any error messages related to the Screen Time Storyteller pipe.
2. Ensure your API keys and GitHub token are correct and have the necessary permissions.
3. Verify that the pipe's directory is in the correct location within your Screenpipe installation.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request to the Screenpipe repository.

## License

This project is licensed under the same license as Screenpipe - see the [LICENSE](LICENSE) file in the Screenpipe repository for details.

## Acknowledgments

- Thanks to the Screenpipe team for providing the underlying screen time tracking functionality.
- Powered by AI models from Anthropic, OpenAI, and Ollama.