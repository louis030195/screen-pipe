import { z } from "zod";

// Define Zod schemas for the API types
const ContentTypeSchema = z.enum(["ocr", "audio", "all"]);

const SearchQuerySchema = z.object({
  q: z.string().optional(),
  limit: z.number().default(20),
  offset: z.number().default(0),
  content_type: ContentTypeSchema.default("all"),
  start_time: z.string().optional(),
  end_time: z.string().optional(),
  app_name: z.string().optional(),
});

const OCRContentSchema = z.object({
  type: z.literal("OCR"),
  content: z.object({
    frame_id: z.number(),
    text: z.string(),
    timestamp: z.string(),
    file_path: z.string(),
    offset_index: z.number(),
    app_name: z.string(),
  }),
});

const AudioContentSchema = z.object({
  type: z.literal("Audio"),
  content: z.object({
    chunk_id: z.number(),
    transcription: z.string(),
    timestamp: z.string(),
    file_path: z.string(),
    offset_index: z.number(),
  }),
});

const ContentItemSchema = z.union([OCRContentSchema, AudioContentSchema]);

const PaginationInfoSchema = z.object({
  limit: z.number(),
  offset: z.number(),
  total: z.number(),
});

const PaginatedResponseSchema = z.object({
  data: z.array(ContentItemSchema),
  pagination: PaginationInfoSchema,
});

// Define types based on the schemas
export type ContentType = z.infer<typeof ContentTypeSchema>;
export type SearchQuery = z.infer<typeof SearchQuerySchema>;
export type ScreenpipeResult = z.infer<typeof ContentItemSchema>;
export type PaginatedResponse = z.infer<typeof PaginatedResponseSchema>;

// Implement the queryScreenpipe function
export async function queryScreenpipe(
  params: SearchQuery
): Promise<PaginatedResponse> {
  const queryParams = new URLSearchParams({
    ...(params.q && { q: params.q }),
    limit: params.limit.toString(),
    offset: params.offset.toString(),
    content_type: params.content_type,
    ...(params.start_time && { start_time: params.start_time }),
    ...(params.end_time && { end_time: params.end_time }),
    ...(params.app_name && { app_name: params.app_name }),
  });

  const response = await fetch(`http://localhost:3030/search?${queryParams}`);

  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`);
  }

  const data = await response.json();
  return PaginatedResponseSchema.parse(data);
}
